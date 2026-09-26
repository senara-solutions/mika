---
issue: 2264
type: fix
title: "qa-review : la porte exige un test négatif mappé-invariant sur les PR dispatch/merge/task_engine"
branch: fix/2264/qa-review-skill-la-porte-exige-un-test-n
status: groomed
---

# Plan — la porte qa-review exige un test négatif nommant l'invariant (mika#2264)

## Invariant (une phrase)

**Une PR qui touche `crates/mika-agent/src/{server,task_engine,tools}/` ne peut pas recevoir
`VERDICT: pass` si son diff n'ajoute aucune assertion négative nommant l'invariant que la PR
pourrait violer — et le relecteur doit avoir nommé cet invariant dans le corps du verdict, pas
seulement constaté la présence de tests.**

Le mot load-bearing est **négative**. Un test positif (« le chemin nominal marche ») est ce que la
cascade du 09-09 avait déjà : #2244 avait des tests, ils passaient, et le merge est parti sous
l'identité du relecteur. Ce qu'aucun de ces tests n'affirmait, c'est « ceci ne doit PAS arriver ».

## Ce que le code dit aujourd'hui (vérifié, `file:line`)

### La porte vit dans `qa-review`, pas dans `self-dev-webhook-qa`

- `skills/bundled/qa-review/system_prompt.md:3` — « You are mika-qa, a specialist reviewer.
  Your job is to review a pull request and produce a structured verdict. » C'est le **producteur**
  du verdict, et le seul acteur qui voie le diff (`3a`, `{{pr_diff}}` injecté par le moteur).
- `skills/bundled/qa-review/system_prompt.md:401` — **Step 3b**, la « liste fermée de 9 checks »
  du ticket, exactement neuf lignes de table : credentials, `unsafe`, `eval`/`exec`, injection SQL,
  erreurs de logique, code mort, gestion d'erreur d'E/S manquante, incohérence de statut TODO,
  refactor comportemental. Aucune n'est un invariant ; aucune n'exige de test.
- `skills/bundled/self-dev-webhook-qa/system_prompt.md:9` — « This message contains a PR review
  body with a `VERDICT:` line posted by mika-qa. […] Your ONLY job is to parse the verdict and act
  on it. » C'est le **consommateur** : il route `pass` vers `pr_merge_with_gate`, `block[*]` vers
  un fix ou une escalade. `skills/bundled/self-dev-webhook-qa/system_prompt.md:13` liste ses outils :
  il n'a **aucun accès au diff**. Il ne peut structurellement pas juger de la présence d'un test.

**Conséquence directe :** écrire la règle dans `self-dev-webhook-qa/system_prompt.md`, comme le
demandait la lettre de l'AC1 du ticket, produirait une règle qu'aucun acteur ne peut appliquer.
Elle vit dans `qa-review/system_prompt.md`. Voir « Divergences avec le corps du ticket » plus bas.

### Le trou mesuré : la classe `CI-deferred` délègue à la CI une question que la CI ne peut pas voir

- `skills/bundled/qa-review/system_prompt.md:290` — la classification des AC prévoit une classe
  **CI-deferred** : *« explicitly defers to CI: "no test regressions", "lints clean", "tests pass" »*.
  Un tel AC est marqué `[⏭️]` en `2.5.6` et **n'est pas vérifié** par le relecteur.
- `skills/bundled/qa-review/system_prompt.md:368` (2.5.7) — `⏭️` compte comme satisfait :
  « All ACs `✅` or `⏭️`: AC verification passes ».

C'est le mécanisme complet de la cascade, et il est plus précis que « la liste est fermée » :
un AC « les tests passent » est *délégué* à la CI, la CI exécute les tests qui existent, et un test
**absent** ne fait échouer aucune CI. La porte a donc une case cochée pour une question que
personne n'a posée. La règle nouvelle doit amender cette classe, pas seulement allonger la table 3b.

### Le point d'accueil existe déjà et son routage aussi

- `skills/bundled/qa-review/system_prompt.md:294` — **2.5.4. Implicit structural AC (always
  applied)** : le prompt sait déjà porter un AC qui n'est écrit dans aucun plan (le no-parallel-plan)
  et le faire *gater*.
- `skills/bundled/qa-review/system_prompt.md:368` (2.5.7) — « Any AC `❌`: `VERDICT: block[ac]`
  (gating) ». Le routage d'un AC implicite non satisfait est donc **déjà** `block[ac]`.
- `skills/bundled/qa-review/system_prompt.md:374` (2.5.8) — `block[ac]` exige une section
  `Plan amendment required:` avec le label littéral `Conflict reason (inferred):`, que
  `crates/mika-agent/src/server/verdict_handler.rs:196` (`"ac" => handle_block_ac`) consomme pour
  router vers un fix sans escalade opérateur.

### `block[test]` n'existe pas et son ajout n'est pas un changement de prompt

- `crates/mika-agent/src/server/verdict.rs:61` — `BLOCK_RE = ^block\[([^\]]+)\]$` et
  `verdict.rs:37` `Verdict::Block(String)` : le *parsing* est générique, `block[test]` serait bien
  parsé.
- `crates/mika-agent/src/server/verdict_handler.rs:195-238` — mais le *routage* est une liste
  fermée : `"ac"`, `"ci"`, `"security" | "pipeline" | "dependency"`, puis
  `verdict_handler.rs:229` `_ => warn!("Unrecognized block[*] verdict subtype — passing through to
  LLM"); VerdictAction::Passthrough`. Un `block[test]` **ne bloque rien de déterministe** : il
  retombe sur le jugement du LLM.
- `skills/bundled/qa-review/skill.toml:28-38` — `[[output.required_tool_arg_suffixes]]` déclare
  une liste fermée de six lignes de verdict acceptées ; `block[test]` n'y est pas, donc l'appel
  `run_gh pr review` serait **rejeté avant spawn** par la garde moteur.
- `skills/bundled/self-dev-webhook-qa/skill.toml:5-14` — le commentaire de mika#2248 mesure la
  conséquence en aval : « ce prompt affirme l'exhaustivité de sa table de dispatch, et mika#2238 a
  mesuré qu'une variante non listée produit un arrêt silencieux ».
- `crates/mika-agent/src/calibration/roles/mika_qa.rs:905-908` — la suite de calibration porte elle
  aussi la liste fermée des verdicts.

**Conséquence :** livrer `block[test]` toucherait cinq surfaces dont deux en Rust moteur. Le ticket
borne explicitement la zone à « skill prompt (pas de code Rust) ». Voir Décision 2.

### Le budget d'octets est la contrainte dure de ce ticket

- `skills/bundled/qa-review/system_prompt.md` mesure **61 498 octets**.
- `skills/bundled/qa-review/skill.toml:10` déclare `max_prompt_size = 65536`.
- `crates/mika-agent/tests/bundled_skills_load.rs:91` — le gate des 95 % (mika#852) **panique**,
  il ne se contente pas d'avertir : `bundled_skills_load.rs:141` `if ratio >= 0.95` →
  `bundled_skills_load.rs:166` `panic!`.
- Seuil : `65536 × 0.95 = 62 259`. **Marge disponible : 761 octets.**
- `crates/mika-agent/src/skills/index.rs:27` — `MAX_PROMPT_SIZE_CEILING = 80 * 1024`, et
  `index.rs:44` `.map(|v| v.min(MAX_PROMPT_SIZE_CEILING))`.

Toute rédaction sérieuse de la règle (règle + invariants candidats + trois exemples négatifs)
dépasse 761 octets. `cargo test` casse si le plafond n'est pas relevé dans le même diff.

### Ce qu'un test peut prouver ici, et ce qu'il ne peut pas

- `crates/mika-agent/tests/eval/skills/mika_arch_fire_disposition_gate.rs:11-25` l'écrit
  noir sur blanc : *« per-skill eval scenarios validate engine handling of a skill's declared
  output shape, not the production prompt's wording. A `MockLlmProvider` […] cannot prove the
  architect's LLM judgment applies the gate »*, et *« the prompt-adherence guarantee […] is owned
  by the calibration suite »*.
- La suite existe pour ce rôle : `Makefile:119` `calibrate-mika-qa`,
  `crates/mika-agent/src/calibration/roles/mika_qa.rs` (7 scénarios), fixtures sous
  `crates/mika-agent/tests/eval/calibration_fixtures/mika-qa/` (`manifest.yaml` + 7 `.md`).

## Divergences avec le corps du ticket (résolues avant grooming)

Trois écarts entre le corps déposé le 09-09 et ce que le code dit. Ils sont **factuels**, pas des
arbitrages de périmètre ; ils ont été corrigés dans le corps du ticket avant le passage architecte,
et la correction est tracée en commentaire sur mika#2264.

| # | Corps déposé | Ce que le code dit | Résolution |
|---|--------------|--------------------|------------|
| D1 | AC1 : la règle va dans `skills/bundled/self-dev-webhook-qa/system_prompt.md` | ce fichier est le **consommateur** du verdict et n'a pas le diff (`self-dev-webhook-qa/system_prompt.md:9,13`) ; la table des 9 checks est en `qa-review/system_prompt.md:401` | la règle va dans `qa-review/system_prompt.md` |
| D2 | verdict `block[test]` | non routé (`verdict_handler.rs:229` → `Passthrough`), rejeté avant spawn (`qa-review/skill.toml:28-38`), arrêt silencieux en aval (`self-dev-webhook-qa/skill.toml:5-14`) ; l'ajouter = code Rust moteur, hors de la zone déclarée | `block[ac]` via l'AC structurel implicite (Décision 2) ; l'ajout d'une sous-variante typée est un ticket séparé |
| D3 | « Zone : skill prompt (pas de code Rust) » | le gate des 95 % panique à 761 octets près (`bundled_skills_load.rs:141,166`) et la garantie d'adhérence appartient à la calibration (`mika_arch_fire_disposition_gate.rs:11-25`) | la zone est **non-moteur** : prompt + `skill.toml` + fixture de calibration ; aucun changement dans `crates/mika-agent/src/` hors `calibration/roles/` |

## Décision 1 — la règle vit en `2.5.4b`, pas seulement dans la table 3b

**Retenu :** une nouvelle sous-étape **2.5.4b — Implicit negative-test AC (path-conditional)**,
immédiatement après `2.5.4` (`qa-review/system_prompt.md:294`).

Pourquoi là plutôt qu'une dixième ligne de la table 3b :

1. **Le routage est gratuit.** `2.5.7` (`:368`) fait déjà `AC ❌ → block[ac]` *gating*, et `2.5.8`
   (`:374`) impose déjà la section que `verdict_handler.rs:196` consomme. Une ligne de table 3b, elle,
   devrait déclarer son propre verdict et n'hériterait d'aucune de ces deux mécaniques.
2. **Le patron existe déjà et est nommé.** `2.5.4` est littéralement « un AC que le plan n'écrit pas
   et que la porte impose quand même ». C'est exactement la forme demandée.
3. **La table 3b reste ce qu'elle est** — une liste de motifs à repérer dans le diff — et gagne une
   ligne de renvoi pour la découvrabilité, sans porter la mécanique.

Le ticket demande aussi que « la liste de 9 checks soit ouverte ». Deux gestes, distincts :
- 3b gagne une dixième ligne : *« PR touchant un chemin sensible sans assertion négative
  nommant l'invariant → voir 2.5.4b »*, verdict `block[ac]`.
- 2.5.4b porte l'ouverture réelle : une liste d'**invariants candidats** explicitement déclarée
  non exhaustive, plus l'obligation faite au relecteur de **nommer l'invariant à risque** de la PR
  qu'il a sous les yeux — y compris quand il n'est dans aucune liste.

## Décision 2 — le verdict est `block[ac]`, l'invariant est nommé dans le corps

**Retenu :** `VERDICT: block[ac]`, avec la section `Plan amendment required:` de `2.5.8` portant
l'invariant nommé en `Conflict reason (inferred):`.

C'est un **renversement de la lettre** de l'AC du ticket (`block[test]`), pas de son intention.
L'intention de Vincent — « fix de porte d'abord », la porte doit mordre — est mieux servie :

- `block[ac]` route déterministiquement vers `handle_block_ac`
  (`verdict_handler.rs:196`) qui dispatche un fix ; `block[test]` retombe sur
  `verdict_handler.rs:229` `Passthrough` — le LLM décide, donc la porte ne mord pas de façon
  déterministe. Une porte non déterministe sur une classe qui vient de coûter quatre incidents est
  pire que pas de porte, parce qu'elle se croit fermée.
- `block[test]` serait **rejeté avant spawn** par `qa-review/skill.toml:28-38` : le relecteur ne
  pourrait même pas poster son verdict, et la revue deviendrait invisible
  (`qa-review/system_prompt.md:540` : sans `run_gh pr review`, « your review is invisible to the
  rest of the system »). C'est-à-dire : un `pass` implicite par silence.
- Sémantiquement, un test négatif manquant **est** un AC structurel implicite non satisfait —
  la même famille que le no-parallel-plan de `2.5.4`.

**Ticket séparé (à déposer, non bloquant) :** faire de `block[test]` une sous-variante typée de
plein droit — bras de match dans `verdict_handler.rs`, ligne dans `qa-review/skill.toml`, branche
de routage dans `self-dev-webhook-qa/system_prompt.md`, entrée dans `calibration/roles/mika_qa.rs`.
Ce n'est justifié que si l'on veut distinguer *en aval* un blocage-test d'un blocage-AC (métrique,
retry différent). Aujourd'hui les deux veulent le même geste : renvoyer au dev.

## Décision 3 — le périmètre de chemins, et le fait qu'il soit déclaré une seule fois

**Retenu :** le périmètre littéral du ticket, `crates/mika-agent/src/server/`,
`crates/mika-agent/src/task_engine/`, `crates/mika-agent/src/tools/`, déclaré **une seule fois**
dans 2.5.4b et référencé depuis 3b.

Le relecteur lit la liste de fichiers déjà en main (`qa_pr_view` de Step 1 rend `files`) — aucun
appel d'outil supplémentaire, donc aucun impact sur le budget de 14 pas
(`qa-review/system_prompt.md:26`).

**Fail-safe : le doute ferme, il n'ouvre pas.** Si le relecteur ne peut pas déterminer les chemins
touchés (liste de fichiers absente, diff en `metadata-only`), la règle 2.5.4b n'est pas « sautée » :
`DEPTH: metadata-only` plafonne déjà le verdict à `hold[review]` (`qa-review/system_prompt.md:75`),
ce qui est plus strict que `pass`. Rien à ajouter — mais 2.5.4b le dit explicitement pour ne pas
laisser le relecteur inventer une troisième voie.

## Décision 4 — la classe `CI-deferred` est amendée, sinon la règle est contournable par construction

**Retenu :** amender `2.5.3` (`qa-review/system_prompt.md:290`) d'une exception d'une phrase :
sur un chemin du périmètre 2.5.4b, un AC de la forme « les tests passent » / « pas de régression »
**n'est pas** CI-deferrable ; il est reclassé **Structural** et vérifié contre le diff.

Sans cet amendement, la règle est contournable sans mauvaise foi : le plan écrit « AC : tests
passent », le relecteur le classe CI-deferred, le marque `[⏭️]`, `2.5.7` compte `⏭️` comme
satisfait, et la porte 2.5.4b se retrouve à devoir agir seule contre une case déjà cochée. C'est le
mécanisme mesuré de la cascade (voir « Le trou mesuré » plus haut), et le fixer est ce qui
distingue ce ticket d'un ajout de paragraphe.

## Décision 5 — le budget d'octets : le plafond monte à 73 728 dans le même diff

**Retenu :** `max_prompt_size = 65536 → 73728` dans `skills/bundled/qa-review/skill.toml`, avec le
commentaire de dérivation, dans **le même commit** que le prompt.

- Nouveau seuil de warn : `73728 × 0.95 = 70 041`. Prompt attendu ≈ 65 500 octets (≈ 89 %).
- Plafond dur respecté : `73728 < 81920` (`index.rs:27`).
- Précédent exact et récent : `self-dev-webhook-qa/skill.toml:5-14` (mika#2248), même geste, même
  raison — un prompt dont la table doit rester exhaustive ne se dégraisse pas en retirant une
  branche.

**La marge est un livrable, pas un effet de bord.** Le budget de rédaction de 2.5.4b + 3b + 2.5.3
est de **~4 000 octets**. S'il devait dépasser 8 500 octets (soit ≥ 70 041 au total), la
remédiation n'est pas « monter encore le plafond » mais scinder par `[dependencies]` — auquel cas
le plan est amendé, pas étiré en silence.

## Décision 6 — ce qui prouve que la porte mord

**Retenu :** une fixture de calibration mika-qa, `negative_test_invariant_gate`, ajoutée à
`crates/mika-agent/tests/eval/calibration_fixtures/mika-qa/` + son scénario dans
`crates/mika-agent/src/calibration/roles/mika_qa.rs`.

C'est le **seul** artefact qui puisse prouver l'adhérence d'un prompt de production : les evals de
skill ne le peuvent pas, et leur propre doc-comment le dit
(`mika_arch_fire_disposition_gate.rs:11-25`). Un test qui se contenterait de `grep` la phrase dans
le prompt prouverait que le texte est présent, pas que la porte mord — ce serait exactement la
faute que ce ticket corrige, commise en la corrigeant.

Le scénario, dans la forme des sept existants (`mika_qa.rs`) :
- **entrée** : un diff synthétique touchant `crates/mika-agent/src/server/`, avec des tests
  *positifs* ajoutés et **aucune** assertion négative, plus un plan dont un AC est « pas de
  régression de tests » ;
- **attendu** : `VERDICT: block[ac]`, section `Plan amendment required:` présente, et le nom d'un
  invariant cité dans le corps ;
- **contrôle négatif, dans le même scénario** : le même diff **hors** périmètre (`crates/mika-cli/`)
  ne déclenche pas la règle — sinon le scénario ne mesure que « le relecteur bloque toujours ».

**Rouge-avant explicite :** le scénario est rouge contre le prompt actuel de `main` — 2.5.4b
n'existe pas, l'AC « pas de régression » est classé CI-deferred (`:290`) et marqué `[⏭️]`, donc
`2.5.7` conclut `pass`. Il est vert après. Cette exécution rouge-avant est un livrable d'AC5, pas
une formalité : elle est ce qui distingue une règle qui mord d'une règle qui décore.

## Acceptance criteria

- **AC1** — `skills/bundled/qa-review/system_prompt.md` porte une sous-étape **2.5.4b — Implicit
  negative-test AC (path-conditional)** qui énonce : une PR touchant
  `crates/mika-agent/src/{server,task_engine,tools}/` dont le diff n'ajoute aucune assertion
  négative nommant l'invariant à risque est un AC structurel implicite **non satisfait** →
  `VERDICT: block[ac]` gating via 2.5.7, avec l'invariant nommé dans la section
  `Plan amendment required:` de 2.5.8. *(= AC1 du ticket, fichier corrigé — cf. D1)*
- **AC2 — la liste est ouverte.** 2.5.4b déclare une liste d'**invariants candidats**
  explicitement non exhaustive, **et** l'obligation faite au relecteur de nommer l'invariant à
  risque de la PR sous ses yeux même lorsqu'il ne figure dans aucune liste. La table 3b
  (`:401`) gagne une dixième ligne renvoyant à 2.5.4b. *(= AC2 du ticket)*
- **AC3 — un exemple négatif par classe.** 2.5.4b porte trois exemples rédigés, un par classe de la
  cascade : identité-de-merge (`mergedBy != reviewer`, mika#2248/#2260), cycle-de-vie-process
  (`superseded → pgid tué`, mika#2263), gate-ignore-label (`blocked → 0 dispatch`). Chaque exemple
  montre la forme de l'assertion négative attendue, pas seulement le nom de l'invariant.
  *(= AC3 du ticket)*
- **AC4 — la classe CI-deferred est fermée sur le périmètre.** `2.5.3` (`:290`) porte l'exception :
  sur un chemin du périmètre 2.5.4b, un AC « tests passent » / « pas de régression » est reclassé
  **Structural** et vérifié contre le diff, jamais marqué `[⏭️]`. *(Décision 4 — sans cet AC, AC1
  est contournable par construction.)*
- **AC5 — non-vacuité (rouge-avant / vert-après).** Le scénario de calibration
  `negative_test_invariant_gate` est ajouté à `calibration/roles/mika_qa.rs` + sa fixture sous
  `tests/eval/calibration_fixtures/mika-qa/` (et son entrée dans `manifest.yaml`). Il est **exécuté
  rouge contre le prompt de `main`** et vert après ; les deux exécutions sont citées dans la PR
  avec la sortie de `make calibrate-mika-qa MODEL=<baseline>`.
- **AC6 — contrôle négatif dans le même scénario.** Le même diff synthétique porté **hors**
  périmètre (`crates/mika-cli/`) ne déclenche pas 2.5.4b : le scénario porte contrôle positif et
  contrôle négatif dans le même appel, sinon il ne mesure que « le relecteur bloque toujours ».
- **AC7 — le budget d'octets tient.** `skills/bundled/qa-review/skill.toml` passe
  `max_prompt_size` à `73728` avec le commentaire de dérivation, dans le même commit que le prompt.
  `cargo test bundled_skills` passe : le prompt est sous 70 041 octets (< 95 % du nouveau plafond)
  et sous le plafond dur de 81 920 (`index.rs:27`).
- **AC8 — aucun changement moteur.** Le diff ne touche aucun fichier de `crates/mika-agent/src/`
  hors `calibration/roles/mika_qa.rs`. En particulier `verdict_handler.rs`, `verdict.rs` et
  `self-dev-webhook-qa/` sont inchangés — la règle est portée par le producteur du verdict et
  routée par des mécaniques existantes. *(= « zone : pas de code Rust » du ticket, précisé en D3)*

## Fire-Disposition

Le livrable détecteur est le scénario de calibration `negative_test_invariant_gate` (AC5) avec son
contrôle négatif (AC6) : **rouge contre le prompt de `main`** — 2.5.4b n'existe pas, l'AC
« pas de régression » est classé CI-deferred (`qa-review/system_prompt.md:290`), marqué `[⏭️]`, et
`2.5.7` conclut `pass` —, **vert après**.

Gate CI : `cargo test` (AC7, gate des 95 % de `bundled_skills_load.rs:141`) est bloquant et
permanent. La calibration (`make calibrate-mika-qa`) n'est pas dans le gate CI par défaut ; son
exécution rouge-avant/vert-après est un **livrable de PR** cité dans le corps, à la manière de
`make calibrate-mika-arch` pour mika#1574.

**Disposition des violations pré-existantes :** aucune. La règle s'applique aux PR **futures** ; ce
diff ne rouvre ni ne re-juge aucune PR déjà mergée. Les quatre incidents de la cascade
(#2248, #2252, #2260, #2263) gardent leurs tickets propres et ne sont pas absorbés ici.

## Phases

1. **Le prompt.** `qa-review/system_prompt.md` : écrire 2.5.4b (règle, périmètre déclaré une fois,
   invariants candidats non exhaustifs, obligation de nommer, trois exemples négatifs, fail-safe
   `metadata-only`) ; amender 2.5.3 (exception CI-deferred) ; ajouter la dixième ligne de 3b.
   Mesurer la taille finale.
2. **Le budget.** `qa-review/skill.toml` : `max_prompt_size = 73728` + commentaire de dérivation
   (dans le même commit que la phase 1 — un commit intermédiaire casserait `cargo test`).
3. **La preuve.** Fixture `negative_test_invariant_gate` + entrée `manifest.yaml` + scénario dans
   `calibration/roles/mika_qa.rs`, contrôle négatif compris.
4. **Rouge-avant.** Exécuter la calibration contre le prompt de `main` (via `git stash` du seul
   fichier prompt, ou un checkout de `main` du prompt) et **capturer la sortie rouge**.
5. **Vert-après.** Ré-exécuter contre le prompt modifié, capturer la sortie verte. Les deux sorties
   vont dans le corps de la PR.
6. **`cargo test` + `cargo clippy`**, et le corps de PR citant les deux exécutions de calibration.

## Hors périmètre

- **`block[test]` comme sous-variante typée** — ticket séparé (Décision 2). Toucherait
  `verdict_handler.rs`, `qa-review/skill.toml`, `self-dev-webhook-qa/system_prompt.md`,
  `calibration/roles/mika_qa.rs`.
- **Un gate structurel côté moteur** (par ex. un check 6 dans
  `crates/mika-agent/src/bin/verify_bundled_skills.rs`, ou une garde
  `required_tool_arg_suffixes` exigeant une section `NEGATIVE-TEST:` dans le corps du verdict).
  C'est la forme qui survivrait à la dérive du prompt — cf. la doctrine « structurel, pas prompt » —
  mais c'est du code moteur, hors de la zone déclarée, et cela n'a de sens qu'une fois la règle
  rédigée et calibrée. **Condition de réveil :** quand une PR du périmètre reçoit `pass` sans test
  négatif *après* le déploiement de ce fix (n=1 suffit — la règle sera alors mesurée non tenue).
- **Étendre le périmètre au-delà des trois chemins du ticket** (par ex. `mika-gateway`,
  `dispatch-lib.sh`). La cascade du 09-09 est entièrement dans les trois chemins ; élargir sans
  évidence ajouterait du faux positif.
- **Re-juger les PR déjà mergées** de la cascade.

## Risques

- **R1 — le relecteur nomme un invariant creux pour passer.** L'obligation de *nommer* est
  satisfiable par une formule vide (« l'invariant de correction »). Atténuation : les trois exemples
  d'AC3 montrent la **forme de l'assertion**, pas seulement le nom, et 2.5.4b exige que l'invariant
  soit cité avec le fichier de test qui l'assert. Mesure de suite : la condition de réveil du
  point « gate structurel » en hors-périmètre.
- **R2 — faux positifs sur les PR de refactor pur** dans le périmètre (renommage, extraction) qui
  n'ajoutent aucun comportement à nier. Atténuation : 2.5.4b n'exige l'assertion négative que
  lorsque le diff **modifie un comportement** dans le périmètre ; un diff purement mécanique nomme
  l'invariant et déclare qu'aucun comportement du périmètre n'est modifié. Cette clause est
  elle-même un risque de contournement — assumée, et c'est la raison d'être de R1.
- **R3 — la calibration est coûteuse et non gatée en CI.** L'AC5 rouge-avant/vert-après est donc un
  livrable humainement vérifiable dans la PR, pas une garde permanente. C'est le même contrat que
  mika#1574 (`mika_arch_fire_disposition_gate.rs:22-25`) ; la garde permanente est `cargo test`
  (AC7), qui protège le budget d'octets mais pas l'adhérence.
- **R4 — dérive du prompt.** Un prompt de 65 Ko qui gagne une règle de plus est un prompt dont
  chaque règle pèse un peu moins. C'est le risque de fond que seul le gate structurel du
  hors-périmètre ferme.

## Ratification architecte (premier passage, `Disposition: READY`)

Session mika-arch `a7ba6655-8c65-445f-a9c8-6023a8761afb`, 2026-09-09. Les trois gates de première
passe passent : Unresolved-Decision (mika#1244), Acceptance-Criteria (mika#1559), Fire-Disposition
(mika#1574). Ce que l'architecte a explicitement tranché, inscrit ici parce qu'un `READY` qui reste
dans une session ne survit pas au dispatch :

1. **D2 ratifié** — `block[ac]` plutôt que `block[test]`. La perte de distinction blocage-test /
   blocage-AC en aval est **assumée** et reste un ticket séparé, pas une dette silencieuse.
2. **AC4 ratifié load-bearing** — la fermeture de la classe CI-deferred sur le périmètre est
   *nécessaire*, pas un ajout de confort : sans elle le fix est contournable par construction.
3. **Placement en `2.5.4b` ratifié** — routage gratuit par 2.5.7/2.5.8, patron existant de 2.5.4,
   et la table 3b reste une liste de motifs sans mécanique.
4. **R1/R2 : l'atténuation suffit pour ce ticket, mais le gate structurel hors périmètre est
   *nécessaire*.** Mot pour mot : « les clauses de prose interprétées restent des clauses de
   prose ». Ce ticket ne clôt donc pas la classe — il pose la règle ; le gate qui la rend
   non-contournable reste dû.
5. **Seuil de réveil n=1 ratifié** — « la doctrine "prompt-only échoue au substrat de la boucle" ne
   tolère pas de tolérance ». Une seule PR du périmètre recevant `pass` sans test négatif après le
   déploiement de ce fix suffit à déclencher le ticket de gate structurel.

## Amendement (AC8) — scope de la harness de calibration (2026-09-09)

Vincent/QA a flaggé `crates/mika-agent/src/bin/calibrate.rs` et `crates/mika-agent/src/calibration/role.rs` comme hors-scope (AC8). **Justification de leur inclusion :** ils sont NÉCESSAIRES pour AC5 (le dogfood red-before/green-after). Sans eux, la calibration n'aurait pas consommé le **prompt de production** de qa-review — la preuve red/green aurait porté sur un prompt-fixture, pas sur le prompt réel que ce PR modifie, donc une preuve creuse. Les changements sont **minimaux** : faire consommer le prompt de production par la harness pour que le scénario `negative_test_invariant_gate` prouve que la VRAIE porte mord. Ils appartiennent donc à #2264 (ils réalisent AC5), pas à un ticket séparé. Si QA/Vincent tranche autrement, les extraire dans un ticket harness dédié est trivial (ils sont isolés).
