# Rendre de la marge au prompt qa-review — découpage du chemin Dependabot

**Ticket :** senara-solutions/mika#3166 (`enhancement`, `p2-normal`, `dispatch:loop`)
**Branche :** `chore/3166/qa-review-rendre-de-la-marge-au-prompt`
**Type :** chore (substrat de prompt + gardes)

---

## 1. Constat, et l'arithmétique qui décide du vol

Mesures prises sous la racine de travail, à HEAD (`76dfeee6`) :

| grandeur | valeur | source |
|---|---|---|
| taille du prompt qa-review | 77 782 octets | `wc -c skills/bundled/qa-review/system_prompt.md` |
| `max_prompt_size` déclaré | 81 920 | `skills/bundled/qa-review/skill.toml:150` |
| `MAX_PROMPT_SIZE_CEILING` | 81 920 (= 80 × 1024) | `crates/mika-agent/src/skills/index.rs:27` |
| seuil de panique (95 %) | 77 824 octets | `bundled_skills_load.rs:142` |
| **marge actuelle** | **42 octets** | 77 824 − 77 782 |
| coût d'AC2 | 1 296 octets | corps du ticket, commit `23e34765` |
| marge exigée par AC1 | > 1 296 octets | AC1 |

Deux voies sur trois sont déjà éteintes, et le dépôt le dit lui-même :

- **Relever le plafond est impossible.** `effective_prompt_limit` applique
  `max_prompt_size.min(MAX_PROMPT_SIZE_CEILING)` (`index.rs:30-34`) et les deux
  nombres sont égaux : il n'y a plus de nombre à écrire. `skill.toml` l'écrit déjà
  en toutes lettres.
- **Le trim bon marché est dépensé.** `skills/bundled/qa-review/skill.toml` porte
  l'historique des relèvements et sa conclusion : *« The next addition takes the
  third remediation the gate offers — shard via `[dependencies]`, extracting a
  sibling skill — and Step 1.6 is the natural seam, being a self-contained path
  with its own entry condition »*. Le seul candidat de trim évident restant — la
  queue de la ligne 46 — est épinglé par `executor.rs:17333-17341` (U3c : la ligne
  doit contenir `not a tooling limitation`, `belongs to the merge gate` et
  `` `CheckClassification::HasFailures` ``). Le ticket et le message de
  `23e34765` refusent tous deux de financer une règle neuve en dégradant cette
  ligne.

Il reste la troisième : **le découpage**. Le ticket la nomme (*« sharding par
`[dependencies]`, décliné deux fois par `skill.toml` comme hors périmètre de leurs
vols »*) et `skill.toml` la désigne **deux fois** comme la suite attendue, en
nommant la couture. Les deux refus précédents étaient des refus de **périmètre**,
pas des refus de fond — et ce ticket EST ce périmètre. Il n'y a donc pas ici de
décision réservée à l'opérateur (mika#3122) : le dépôt a déjà tranché le
mécanisme, le ticket l'énumère comme l'attendu, et ce plan l'exécute.

**La couture.** Step 1.6 (chemin Dependabot, mika#1729) occupe les lignes 139 à
215, soit **10 324 octets**. Elle est auto-contenue : condition d'entrée propre
(`author.login == "dependabot[bot]"` ou `"app/dependabot"`, `author` étant un
OBJET — mika#2519), saut explicite des Steps 2 et 2.5, bloc `DEP-REVIEW:`
obligatoire, requête d'avis `gh api /advisories`, ligne `API-SURFACE:` du saut
majeur (mika#2519) et compilation dans un worktree créé (mika#2565) avant fin de
tour.

**Arithmétique du vol** (ce qui rend AC1 atteignable) :

```
77 782  prompt actuel
−10 324  Step 1.6 (lignes 139–215) part dans le shard
+   900  talon d'orientation laissé en place (D5)
+ 1 296  AC2 : deux puces d'abstention CI + ligne canonique CI-Status:
=  69 654  octets après le vol
```

Marge résultante : 77 824 − 69 654 = **8 170 octets**, soit 6,3 fois le plancher
d'AC1. Ratio de jauge : 85,0 % (sous les 95 %). Le shard pèse ≈ 11 100 octets
(Step 1.6 + son en-tête de portée) contre un plafond déclaré de 32 768 : ratio
33,9 %, marge ≈ 20 000 octets.

**Pourquoi le coût de contexte est nul, et pas seulement faible.**
`max_prompt_size` est **par skill, jamais sur la somme** — c'est ce qui fait
sortir les 10 324 octets de la jauge. Mais le texte, lui, reste injecté sur
exactement les mêmes tours qu'aujourd'hui : `match_skills()`
(`matcher.rs:96-200`) fait sa passe 1 sur `always_on` — donc `qa-review` — puis
un BFS sur les dépendances **sortantes**, qui atteint le shard ; et
`callback_safe_skills()` (`skills/mod.rs:1056`) amorce sur `always_on` et suit
les mêmes arêtes sortantes. Le même octet, les mêmes tours, deux jauges. C'est
le précédent exact de `qa-review-build-callback` (+19 330 octets/tour, déjà
assumé).

---

## 2. Décisions

### D1 — Un shard `qa-review-dependabot`, pas un trim

Step 1.6 part dans `skills/bundled/qa-review-dependabot/`. Motivation ci-dessus :
les deux autres remédiations sont éteintes, et celle-ci est celle que le dépôt
désigne.

### D2 — Câblage calqué sur `qa-review-build-callback`, pas inventé

Le frère vit à `always_on = false`, `max_prompt_size = 32768`,
`dependencies = ["qa-review"]`, et c'est `qa-review` qui le déclare dans ses
propres `dependencies` — l'arête **sortante** qui porte l'atteignabilité
(mika#2355 B1). Le shard reprend cette forme à l'identique :

```toml
[skill]
name = "qa-review-dependabot"
description = "Dependabot dependency-PR review path for qa-review (mika#1729)"
version = "0.1.0"
always_on = false
max_prompt_size = 32768
dependencies = ["qa-review"]

[triggers]
keywords = ["dependabot", "dependency bump", "advisory"]
```

et `qa-review/skill.toml` ajoute `"qa-review-dependabot"` à ses `dependencies`.

**`always_on = true` a été envisagé et refusé.** Il rendrait le shard atteignable
même sur les cinq déclencheurs silencieux que `safe_always_on_skills()` sert sans
résoudre les dépendances (`Heartbeat`, `Reflection`, `Reminder`, `SkillRun`,
`CuratorReview` — `agent_loop/mod.rs:7856-7860`), là où l'arête seule ne porte
pas. Mais **aucun tour de revue ne vit sur ces déclencheurs** : une revue arrive
par webhook conversationnel ou par callback, et les deux chemins résolvent les
arêtes sortantes. Dévier du motif frère pour couvrir une population vide
coûterait une forme de plus à expliquer, et retirerait au passage l'intérêt de la
garde d'atteignabilité de D7 — qui mord justement parce que l'atteignabilité
repose sur l'arête.

### D3 — Le shard ne déclare AUCUN `required_tools`

Les quatre outils du chemin Dependabot (`qa_pr_view`, `run_gh`, `run_shell`,
`build_mika`) sont **déjà** dans `[constraints] required_tools` de `qa-review`,
donc l'armement de la garde reste bit pour bit celui d'aujourd'hui. Les déclarer
sur le shard l'aggraverait au lieu de le préserver : le shard étant injecté sur
**chaque** tour mika-qa par l'arête, `required_tools = ["build_mika"]` exigerait
un appel `build_mika` sur une revue ordinaire — un refus massif de tours sains.
Même raisonnement pour `[[output.required_tool_arg_suffixes]]` : le verdict du
chemin Dependabot sort par le même `run_gh pr review` que les autres, et
l'alphabet fermé de `qa-review` le couvre déjà (dont `block[dependency]`).
`[context.pr_diff]` reste également sur `qa-review` seul.

### D4 — Un en-tête de portée, repris du frère

Le prompt du shard ouvre sur un bloc cité calqué sur
`qa-review-build-callback/system_prompt.md:1-7` : la résolution de dépendances
n'est pas conditionnelle, donc le fichier est injecté sur **tous** les tours
qa-review, Dependabot ou non. Sans en-tête, un tour ordinaire lit « cette étape
remplace le pipeline plan-AC » hors contexte — la régression silencieuse exacte
que mika#2355 a nommée sur le frère (un `pass` posé sans diff lu). L'en-tête dit
donc, en première ligne lisible : **si `author.login` n'est pas Dependabot, rien
de ce fichier ne s'applique, et les Steps 2 / 2.5 / 3 de `qa-review` tournent en
entier.**

### D5 — Un talon d'orientation reste dans `qa-review`

Le prompt principal garde, à la place des 77 lignes, un talon de ≈ 900 octets qui
(a) nomme la condition d'entrée (`author.login == "dependabot[bot]"` ou
`"app/dependabot"`, `author` est un objet — mika#2519), (b) renvoie au prompt
`qa-review-dependabot` **déjà présent dans le même tour**, et (c) conserve la
phrase opérante que le reste du dépôt cite : *« Do NOT run Steps 2/2.5 for a
Dependabot PR »*. Sans talon, un tour Dependabot n'a aucun point d'entrée depuis
le pipeline numéroté.

### D6 — AC2 est restauré **verbatim**, par extraction git

Les deux puces et la ligne canonique `CI-Status:` ne sont pas réécrites de
mémoire : elles sont extraites du commit qui les a introduites (`0affd422`) et
replacées après la ligne 46, dans `### Data Integrity Rules`, exactement où
`23e34765` les a retirées (`system_prompt.md | 2 --`). Forme d'extraction
(mika#2548), seule sur sa ligne :

```
git show 0affd422:skills/bundled/qa-review/system_prompt.md > .pilot-scratch/qa-review-0affd422.md
```

Leur rédaction d'origine est déjà écrite pour **ne pas déclencher le détecteur
sur elle-même** (négateurs, plus l'exemption de préfixe `ci-status:`), ce qui est
la condition pour que `CI_ATTESTATION_PROMPT_ALLOWLIST` reste vide — voir D7.

### D7 — Trois détecteurs, dont deux neufs, et un scan à ré-élargir

1. **Plancher de marge (AC1)**, nouveau, dans
   `crates/mika-agent/tests/bundled_skills_load.rs` : la marge de `qa-review`
   sous son seuil de panique doit rester ≥ 1 296 octets, le nombre étant le coût
   mesuré de son premier client (PR #3164). Écrit **nommément sur qa-review**, et
   non sur la classe « tout prompt bundled » : la version de classe atterrirait
   rouge à cause de `self-dev` (68 696 octets sous un seuil de 70 041, soit
   1 345 octets de marge — 49 de jeu au-dessus du plancher), qui n'est pas le
   sujet de ce ticket et dont la correction est un autre vol. Un détecteur de
   classe livré rouge par un tiers exige une exception nommée pour ce tiers :
   c'est l'allowlist que mika#2309 condamne. Voir § *Hors périmètre, mesuré*.
2. **Atteignabilité du shard**, nouveau, dans
   `crates/mika-agent/src/bundled_skills.rs`, à côté de
   `mika2355_every_bundled_callback_handler_is_reachable_on_a_callback_turn` et
   réutilisant son `registry_from_bundled_manifests()` (donc aucun helper neuf) :
   `qa-review-dependabot` doit apparaître dans `callback_safe_skills()`. Le
   détecteur de classe existant ne peut structurellement pas le voir — son
   prédicat est le suffixe `-callback`.
3. **Scan d'armement CI, ré-élargi**, dans
   `crates/mika-agent/src/evidence/guards.rs:8937-8992` : le test
   `le_prompt_de_qa_review_ne_declenche_pas_le_detecteur` scanne
   `include_str!`-ement `PROMPT_QA_REVIEW`. Après le découpage, 10 Ko de prose
   CI-adjacente (« distinct-from-CI », « CI only proves the new version
   compiles », « not excluded from CI ») quittent cet ensemble : le détecteur
   resterait vert **pour la mauvaise raison**. Il prend donc un second
   `include_str!` sur le prompt du shard et boucle sur les deux. Même geste pour
   le chemin cité par la table en prose de `guards.rs:3928`, qui désigne *«
   `qa-review/system_prompt.md` Step 1.6 »* comme l'un des deux étages de
   l'exemption de plan Dependabot.

### D8 — Ce que ce vol ne fait pas

- Il ne **déploie** pas. `skills/bundled/` est lu au build
  (`build.rs` → `BUNDLED_SKILL_MANIFESTS`) et `~/.mika/skills/` est une
  projection du binaire : aucun agent ne voit ce découpage avant `make deploy`
  (mika#2340). La PR livre le substrat ; la mise en service est un geste
  opérateur.
- Il ne touche **ni** à la ligne 46, **ni** à la ligne 5 (l'énumération
  `pull_request.opened|synchronize|review_requested|ready_for_review` dont dépend
  le terme T2 de
  `mika2212_every_pull_request_action_routed_to_qa_is_covered_by_a_handler`,
  `mika-gateway/src/github.rs:1972-2096`), **ni** aux fenêtres 2.5.3 et
  Data-Integrity épinglées par `executor.rs:17274-17342`, **ni** à Step 1.5 (dont
  `skills/bundled/_shared/tests/test_rescue_closes_guard.sh:217-230` épingle
  quatre aiguilles).
- Il ne corrige pas la marge de `self-dev`.

---

## 3. Ordre de commit et plancher du vol

Un seul vol (§ *Taille estimée*). Ordre de commit, chaque étape laissant l'arbre
compilable :

1. `skills/bundled/qa-review-dependabot/{skill.toml,system_prompt.md}` — le shard
   complet (en-tête de portée D4 + Step 1.6 déplacé verbatim).
2. `skills/bundled/qa-review/system_prompt.md` — Step 1.6 → talon (D5), puis les
   deux lignes d'AC2 restaurées après la ligne 46 (D6).
3. `skills/bundled/qa-review/skill.toml` — l'arête sortante + la note de vol qui
   remplace la doctrine « la prochaine addition devra sharder » par ce qui a été
   fait et ce qu'il reste de marge.
4. `crates/mika-agent/src/well_known_agents.rs` — `"qa-review-dependabot"` dans
   `MIKA_QA_IDENTITY` (lignes 321-324) ; les attentes de test (2732-2733,
   2847-2848, 3082-3090) et le compte de la ligne 3292 passent de 19 à 20. Un
   skill bundled neuf est **refusé par défaut** hors allowlist, et une dépendance
   désactivée en milieu de chaîne casse son sous-arbre : sans cette étape, le
   shard est injecté par personne et le chemin Dependabot devient muet
   (mika#2355).
5. Repointage des épinglages : les sept tests Step-1.6 et
   `assert_step_1_6_exists` de
   `crates/mika-agent/tests/qa_review_executes_repo_guards.rs` lisent désormais
   le prompt du shard via un helper frère de `qa_review_prompt()` (ligne 49).
   **Mêmes aiguilles, nouveau fichier** — aucune assertion n'est retirée ni
   affaiblie (AC3).
6. Détecteurs : plancher de marge, atteignabilité, scan CI ré-élargi (D7).

**Plancher du vol** (ce en deçà de quoi la PR n'est pas mergeable) : étapes 1 à 5.
Le découpage sans l'allowlist de l'étape 4 livre un chemin muet ; le découpage
sans l'étape 5 livre des épinglages rouges. L'étape 6 est la seule séparable, et
elle n'est pas séparée : AC1 se mesure par un test.

**Sites périphériques à vérifier au vol, sans changement attendu :**
`crates/mika-agent/tests/skill_prompt_user_prescription_guard_2024.rs` (découvre
les prompts par `read_dir` sur `skills/bundled`, donc absorbe le shard tout seul ;
son ancre nommée reste `qa-review/system_prompt.md`),
`crates/mika-agent/src/qa_reviewed_head.rs:1012-1110` (`PROMPTS_ECRIVANTS` est
indemne **tant que** le shard ne porte aucun `record_reviewed_head(` — il n'en
porte pas), `src/perimeter/tests.rs:104`,
`src/calibration/roles/mika_qa.rs`, `src/builtin_handlers.rs:13445`,
`src/read_agent_file.rs:568`, `crates/mika-common/src/home.rs:633, 1433-1434, 1807`.

---

## 4. Contrat de vérification

| ce qui est vérifié | comment |
|---|---|
| AC1 — marge > 1 296 octets | `cargo test -p mika-agent --test bundled_skills_load` (jauge 95 % **et** nouveau plancher) |
| AC1 — pas de saut de falaise | `bundled_skills_load_without_oversized_prompts` (porte 100 %) |
| AC2 — les deux puces et `CI-Status:` sont revenues | `git diff origin/main -- skills/bundled/qa-review/system_prompt.md` montre les deux lignes de `0affd422` ; le scan d'armement de `guards.rs` reste vert avec `CI_ATTESTATION_PROMPT_ALLOWLIST` **vide** |
| AC3 — aucun épinglage affaibli | `cargo test -p mika-agent --test qa_review_executes_repo_guards` (les 7 tests 1.6 + les 9 autres), `cargo test -p mika-agent skills::executor::` (mika2423 U3a/U3b/U3c), `cargo test -p mika-gateway mika2212`, `bash skills/bundled/_shared/tests/test_rescue_closes_guard.sh` |
| structure du nouveau bundle | `make verify-bundled-skills` (fichiers requis, parse du manifeste, résolution des handlers, cohérence `required_tools`, **cohérence des allowlists d'identité**) |
| atteignabilité | le détecteur D7.2 + `cargo test -p mika-agent well_known_agents` (compte à 20) |
| pipeline | `cargo test`, `cargo clippy`, `cargo fmt --check`, `bash scripts/verify-pipeline.sh origin/main` |

---

## 5. Fire-Disposition

Ce plan livre des détecteurs (D7) : la section est due (mika#2306). **Option (a)
— exception nommée en allowlist — pour les trois, avec un ensemble d'exceptions
livré VIDE et son vide asserté**, qui est l'idiome canonique de ce dépôt
(`CI_ATTESTATION_PROMPT_ALLOWLIST`, `PROMPT_OUTCOME_ALLOWLIST`,
`SETTABLE_KEYS_NOT_DB_BACKED_EXEMPT` : tous livrés vides, tous auto-nettoyants).

- **Plancher de marge (D7.1).** Atterrit **vert** : la marge post-vol est de
  8 170 octets contre un plancher de 1 296. Aucune violation existante, donc
  aucune exception à nommer — l'ensemble est vide par construction, et le
  détecteur porte dans son message d'échec les trois remédiations de la jauge
  plus la mention que la première (relever le plafond) est éteinte. Quand il
  rougira, le geste est de **rendre de la marge**, jamais d'abaisser 1 296 :
  le nombre est le coût mesuré d'un client réel.
- **Atteignabilité (D7.2).** Atterrit **vert** : l'arête sortante est posée dans
  le même vol. Aucune exception. Le frère mika#2355 en porte une
  (`self-dev-callback`, ticket mika#2356) avec son assertion de péremption ; le
  nouveau détecteur n'en a aucune, et c'est sa forme définitive — il nomme un
  skill et une arête.
- **Scan d'armement CI (D7.3).** Pas un détecteur neuf mais un détecteur dont la
  population est **ré-élargie** : il est vert aujourd'hui, il doit rester vert
  après le retour des deux puces d'AC2, et `CI_ATTESTATION_PROMPT_ALLOWLIST`
  reste **vide**. Son doc-comment prescrit déjà le geste et ce plan ne le change
  pas : quand il rougit, **on corrige la ligne du prompt**, on n'ajoute pas
  d'exception. Si les lignes extraites de `0affd422` le faisaient rougir, c'est
  une erreur d'extraction (elles étaient vertes à leur mergé) — pas un motif
  d'exception.

Ni (b) ni (c) : rien n'est livré désarmé, et aucune halte n'est à remonter.

---

## 6. Hors périmètre, mesuré

`skills/bundled/self-dev/system_prompt.md` pèse 68 696 octets sous un
`max_prompt_size = 73728`, soit un seuil de panique à 70 041 et une marge de
**1 345 octets** — 49 octets au-dessus du plancher que ce ticket rend exigible
pour qa-review, et le second plus gros prompt du dépôt. Son plafond n'est pas
encore au `MAX_PROMPT_SIZE_CEILING` (73 728 < 81 920), donc la remédiation « lever
le plafond » lui est encore ouverte, une fois. Ce plan ne le touche pas et c'est
pourquoi D7.1 est nommé plutôt que de classe. Un ticket de suivi est à ouvrir par
l'opérateur ; ce vol ne l'ouvre pas et ne le traite pas.

---

## 7. Definition of Done

- [ ] `skills/bundled/qa-review-dependabot/` existe avec `skill.toml` et
      `system_prompt.md`, l'en-tête de portée en tête du prompt, et Step 1.6
      déplacée verbatim.
- [ ] `qa-review/skill.toml` déclare `"qa-review-dependabot"` dans ses
      `dependencies`, et sa note de vol remplace la doctrine « la prochaine
      addition devra sharder » par l'état réel.
- [ ] `qa-review/system_prompt.md` porte le talon d'orientation et les deux
      lignes d'AC2, et sa taille est ≤ 70 000 octets.
- [ ] `MIKA_QA_IDENTITY` contient `qa-review-dependabot` et le compte de skills
      annoncé par `well_known_agents.rs:3292` est passé à 20.
- [ ] Les sept tests Step-1.6 et `assert_step_1_6_exists` lisent le prompt du
      shard, avec les mêmes aiguilles.
- [ ] Les trois détecteurs de D7 sont verts, allowlists vides.
- [ ] `make verify-bundled-skills`, `cargo test`, `cargo clippy`,
      `cargo fmt --check` et `bash scripts/verify-pipeline.sh origin/main`
      passent.
- [ ] Le corps de PR nomme `make deploy` comme précondition de mise en service
      (mika#2340).

---

## Acceptance criteria

- AC1 : la marge du prompt qa-review sous la jauge dépasse 1 296 octets, mesurée par le test de jauge.
- AC2 : les deux puces d'abstention CI et la ligne canonique `CI-Status:` retirées par #3164 sont réintroduites dans le prompt.
- AC3 : aucun test d'épinglage existant n'est affaibli.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/qa-review-dependabot/skill.toml` | 40 |
| `skills/bundled/qa-review-dependabot/system_prompt.md` (en-tête + Step 1.6 déplacée) | 95 |
| `skills/bundled/qa-review/system_prompt.md` (talon + 2 lignes AC2) | 14 |
| `skills/bundled/qa-review/skill.toml` (arête + note de vol) | 18 |
| `crates/mika-agent/src/well_known_agents.rs` (allowlist + compte 19→20) | 8 |
| `crates/mika-agent/tests/qa_review_executes_repo_guards.rs` (helper + repointage de 8 sites) | 28 |
| `crates/mika-agent/src/evidence/guards.rs` (scan CI ré-élargi + chemin cité) | 14 |
| `crates/mika-agent/tests/bundled_skills_load.rs` (plancher de marge) | 30 |
| `crates/mika-agent/src/bundled_skills.rs` (atteignabilité du shard) | 18 |

Total estimé : 265 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 265 < 279, et la marge de 14 lignes est tenue par
trois renoncements explicites, chacun chiffré : le détecteur de marge est nommé
et non de classe (une version de classe coûterait l'exception `self-dev` et sa
mécanique de péremption, ≈ +25 lignes, pour arriver rouge — D7.1) ; le détecteur
d'atteignabilité réutilise `registry_from_bundled_manifests()` au lieu de
généraliser l'heuristique de nom de mika#2355 en axe déclaratif de manifeste, ce
que son propre ticket de suivi mika#2356 porte déjà (≈ +45 lignes) ; et le scan
d'armement CI prend un second `include_str!` plutôt qu'une découverte dynamique
des prompts bundled (≈ +20 lignes). L'essentiel du volume est un **déplacement**
de 77 lignes de prose déjà écrites, dont le coût en tours est une extraction et
une écriture, non une rédaction. Aucun reste n'est reporté : les trois ACs sont
servis par ce vol, donc aucune ligne `Tracked in:` n'est due.
