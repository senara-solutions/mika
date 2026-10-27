# fix(2926) — mika-qa épuise ses 20 pas en `run_shell` et finit sans verdict

> Ticket : senara-solutions/mika#2926 — `bug`, `p1-important`, `dispatch:loop`,
> `loop-substrate`, jalon *Substrat de boucle*. Merge : Vincent.

## Contexte

13 tours de mika-qa ont été coupés au plafond de steps entre le 2026-10-23T13:45Z
et le 2026-10-26T18:10Z, dont 4 tours de callback. `run_shell` domine chacun des
13 : 9 à 28 appels sur des totaux de 20 à 31. `qa_pr_view`, l'outil fait pour lire
la PR, est appelé une fois. Sur mika#2920 deux passages consécutifs sont morts sur
la tête `0a55d28a` — le tour `synchronize` 4f94fbc8 sans même lancer de build, puis
le tour de callback 0f07c520 d'un build qui avait réussi — et la seule revue de la
PR reste un COMMENTED sur une tête antérieure.

Le ticket écarte explicitement `qa_callback_verdict` / `no_target_stamp` comme
cause : le build 59f53b3f était né d'un `mika ask` et non du chemin webhook, donc
sa metadata ne portait pas `qa_review_pr_target`, et
`read_qa_review_pr_target` (`task_engine/dispatcher.rs:405`) décrit ce cas comme
nominal. Le filet a correctement refusé de deviner une cible. **Ce vol n'y touche
pas.**

## Le relevé (AC1), et la lecture qu'il permet

### 1. La part mesurée en production

Le tableau du ticket, qui est la mesure et que ce plan ne réécrit pas :

| horodatage (UTC) | session | `total_calls` | `run_shell` | `qa_pr_view` | part de `run_shell` |
|---|---|---|---|---|---|
| 2026-10-23T13:45:32 | 17bac7ec | 25 | 22 | 1 | 88 % |
| 2026-10-23T15:04:24 | callback-195f1aa0 | 22 | 17 | 1 | 77 % |
| 2026-10-23T18:43:02 | callback-4465e50b | 22 | 20 | 1 | 91 % |
| 2026-10-23T22:18:21 | callback-12c8688d | 28 | 28 | 0 | 100 % |
| 2026-10-25T12:39:48 | ab7aba49 | 31 | 26 | 1 | 84 % |
| 2026-10-26T15:19:16 | 4f94fbc8 | 22 | 15 | 1 | 68 % |
| 2026-10-26T18:10:28 | callback-0f07c520 | 26 | 23 | 1 | 88 % |

Sept tours sur treize, 68 % à 100 % du budget en `run_shell`.

### 2. Pourquoi la catégorie des commandes n'est pas dans le log, et où elle est

`max_steps_tool_census` **ne porte que des noms d'outils**, par choix explicite et
documenté — `max_steps_census.rs` le dit à son émetteur :

> « La carte ne porte que des **noms d'outils** — jamais `input_summary` ni
> `output_summary`. Aucune valeur d'entrée ou de sortie n'y transite, donc aucun
> passage par `secret_scrubber` n'est requis : c'est une propriété du choix de
> champs, pas une vérification à refaire. »

Les deux seuls sites qui logguent une **forme de commande** sont des WARN de
timeout (`tool_execution/dispatch.rs:564` et `:594`, champ `input_excerpt`) ; le
site nominal est un `debug!(tool = %name, …)` qui ne porte pas la commande et
n'est pas émis en production. La forme de la commande vit donc dans
`tool_calls.input` en base (scrubée depuis v28→v29), et la requête qui produit le
relevé par catégorie est :

```sql
-- sur l'hôte : sqlite3 ~/.mika/data/mika.db
SELECT substr(json_extract(input, '$.command'), 1, 60) AS forme, COUNT(*) AS n
FROM tool_calls
WHERE agent_id = 'mika-qa' AND name = 'run_shell'
  AND created_at >= '2026-10-23T13:00:00Z'
GROUP BY 1 ORDER BY n DESC;
```

**Ni ce pilote de groom ni le pilote d'implémentation ne peuvent l'exécuter**, et
c'est vérifié, pas supposé : `/var/log/mika/` n'existe pas dans le bac à sable de
dispatch, `~/.mika/data/` n'y contient que `pilot-transcripts/` (pas de
`mika.db`), et `curl` vers `127.0.0.1:8080` est refusé par la permission-policy
(`no matching policy rule -- denied by default`). La requête ci-dessus est donc
une **confirmation au merge**, pour Vincent, d'une ligne — jamais un préalable à
l'implémentation.

### 3. Le relevé du prescripteur, catégorie par catégorie

Ce que le dépôt porte et qui est lisible intégralement : les `run_shell` que les
deux prompts **prescrivent**. Six catégories, avec leur compte prescrit et leur
origine dans le prompt :

| # | catégorie | sites | prescrit par tour | borné par le prompt ? |
|---|---|---|---|---|
| C1 | exécution des guards du dépôt dans un worktree jetable | Step 2B | 1 | **oui** — « One `run_shell` call, cleanup included » |
| C2 | lecture du plan (`cat <worktree>/<plan-path>`) | Step 2.5.1 ; callback item 1 | 1 par tour, re-lecture **obligatoire** au callback | oui |
| C3 | dérivation / création du worktree (`fetch`, `worktree add`, `checkout <head_sha>`) | Step 3e.2, 5c.ii, l. 620 | 0–2 | oui |
| C4 | lecture de fichiers du dépôt pour vérifier une AC (`cat`, `sed -n`) | Step 3 « Documentation » (l. 422), Step 3e | 0–n | **NON** |
| C5 | recherche dans le dépôt (`grep`, `--name-only`, call sites) | Step 2.5.4, Step 5b | 1–n | **NON** |
| C6 | exécution d'AC comportementales (`<worktree>/target/release/mika …`) | Step 4 ; callback l. 55 | 0–3 | oui |

Somme prescrite nominale : **3 à 9+**, contre 15 à 28 mesurés. Et la propriété
décisive est dans la dernière colonne : **sur six catégories, deux n'ont aucune
borne dans le prompt** — et ce sont exactement celles que le ticket nomme comme
candidates (« lecture du diff », « recherche dans le dépôt »). Sur un tour qui
hésite, C4 et C5 sont les seules qui peuvent croître sans qu'aucune règle du
prompt ne s'y oppose.

### 4. Le fait indépendant, et le plus fort

**Le prompt annonce un plafond que le moteur ne tient pas.**
`qa-review/system_prompt.md:17` dit :

> « You have a maximum of **14** tool steps per turn. Plan carefully: »

Le moteur en donne **20** (`planning/policy.rs`, `MAX_TOOL_STEPS = 20` et
`MAX_CALLBACK_TOOL_STEPS = 20`). Le tableau de budget qui suit cette phrase somme
à 6 au minimum et **13** au maximum (1+1+2+1+1+2+3+1+1), donc le prompt est
cohérent avec son propre « 14 » et c'est le nombre lui-même qui est faux depuis
une valeur de `MAX_TOOL_STEPS` antérieure. Le nombre vit à deux endroits et rien
ne les lie : c'est la classe de dérive que ce dépôt refuse ailleurs par une garde
structurelle, et nulle part ici.

Ce fait est vérifiable dans le dépôt, sans la base de production, et il porte
seul la moitié de la décision ci-dessous.

## La décision : (c), les deux — et pourquoi dans cet ordre

**(a) relever le plafond pour mika-qa seul, et (b) resserrer les deux prompts.**
Pas l'un sans l'autre, et la raison est dans la mécanique de vérification autant
que dans la mesure :

- **(a) seul déplace le problème**, comme le ticket l'avertit. Le relevé du § 3
  dit pourquoi : deux catégories sur six sont sans borne, donc un plafond plus
  haut leur donne simplement plus de place.
- **(b) seul n'est pas vérifiable.** AC2 demande un corpus rejoué. Le harnais
  d'eval sert les réponses du modèle depuis `MockLlmProvider` : le mock **ne lit
  pas le prompt**. Un resserrage de prose ne peut donc produire aucune différence
  observable entre la « forme d'avant » et la « forme d'après » d'un test rejoué.
  Ce qu'AC2 peut discriminer, c'est un changement de **mécanique** — et ici c'est
  le plafond.
- Donc **(a) ferme AC2 et AC3**, et **(b) ferme la clause « l'épuisement peut se
  déplacer »**, sous la sonde d'AC4 qui est le seul juge des deux.

### Le nombre, et sa chaîne de justification

**30 steps pour mika-qa**, 20 pour tous les autres agents. La chaîne :

1. Le prescripteur demande **13** steps dans son cas le plus chargé (somme du
   tableau de budget, § 4).
2. La mesure dit **15 à 28** `run_shell` par tour coupé, soit 1,2 à 2,2 fois la
   prescription *totale* en `run_shell` seul.
3. Le maximum de `total_calls` observé est **31** ; 30 couvre donc la
   quasi-totalité de la population mesurée tout en restant un plafond — au-delà
   de 30 un tour n'est plus serré, il est pathologique, et le couper est le bon
   comportement.
4. 30 / 13 ≈ 2,3 : la marge retenue est du même ordre que le ratio mesuré, pas un
   arrondi confortable.

### Le risque nommé : l'épuisement peut se déplacer vers l'enveloppe de temps

L'enveloppe d'un tour est de 300 s (`DEFAULT_AGENT_TOTAL_TIMEOUT_SECS`, réglable
par `MIKA_AGENT_TOTAL_TIMEOUT_SECS` et par provider depuis mika#2189). Les 13
tours mesurés ont été coupés par le **budget de steps**, pas par l'enveloppe —
sinon ils auraient pris le chemin `DeadlineExceeded`, qui n'émet pas
`max_steps_tool_census`. Donc à 20 steps le step moyen de ces tours est
nécessairement sous 15 s. Au même rythme, 30 steps demandent jusqu'à 450 s :
**la coupure peut se déplacer vers `DeadlineExceeded`.**

Ce vol **ne relève pas l'enveloppe** — c'est un second axe, un levier opérateur
existe, et l'élargir sans mesure referait l'erreur que ce plan évite. Ce vol fait
la seule chose qui rende le déplacement visible : **la sonde d'AC4 compte les deux
causes**. Une sonde qui ne compterait que `max_steps_tool_census` lirait un
déplacement vers l'enveloppe comme un succès — le silence n'est pas une réussite,
et c'est la correction que ce plan apporte à la sonde telle que le ticket l'avait
formulée.

### Le resserrage (b), sous une contrainte de taille qui le cadre

`qa-review/system_prompt.md` mesure **77 292 octets** et son
`max_prompt_size` est à **81 920**, c'est-à-dire sur `MAX_PROMPT_SIZE_CEILING`
(`skills/index.rs:27`). Le gate à 95 % **panique** (`tests/bundled_skills_load.rs`)
à 77 824 : il reste **532 octets**, et le `skill.toml` déclare que la remédiation
par élévation est épuisée — la prochaine addition trimme ou sharde.

Le resserrage est donc **à somme quasi nulle**, et chaque octet est nommé :

| geste | Δ octets |
|---|---|
| `14` → `30` à la ligne 17 | +0 (deux caractères, même longueur) |
| borne sur C4 + C5 : au plus 2 `run_shell` de lecture/recherche hors C1–C3, et sortie bornée en `hold[review]` nommant ce qui manquait | ≈ +420 |
| trim compensatoire : le *why* historique de 2B (« Measured cost on `mika` (3323 tracked files)… without anything saying so », l. 234) — ~330 octets dont la surface qui les porte déjà est `skills/executor.rs` (`BUILD_FLOOR_SECS`) et `crates/mika-agent/CLAUDE.md` § *Per-Tool Skill Budget* | −330 |

Net ≈ **+90 octets**, soit ~77 382, et ~440 octets de marge sous le gate. C'est la
discipline que le `skill.toml` énonce lui-même : *le prompt garde la règle, le
« pourquoi » va sur la surface qui le lit.* La vérification est un `wc -c` plus le
test qui panique.

Côté `qa-review-build-callback/system_prompt.md` (22 564 octets pour un
`max_prompt_size` de 32 768 — ~8 500 octets de marge), un seul ajout : la
re-lecture du plan y est **obligatoire** et le chemin du worktree est **dérivé par
le modèle**, donc un chemin faux ouvre une boucle de récupération que rien ne
borne. La règle la borne : deux tentatives de lecture du plan, puis
`block[pipeline]` — ce que le prompt prescrit déjà comme disposition
(« plan unreadable in callback »), désormais avec un compteur.

## Requirements

### R1 — Un plafond de steps par agent, résolu à un seul site

`crates/mika-agent/src/planning/policy.rs` gagne :

```rust
/// Le plafond de steps d'outil d'un tour, par agent (mika#2926).
///
/// `default` est le plafond du mode (`LoopMode::max_steps()`) : conversation,
/// callback ou team. Cette fonction ne le remplace que pour les agents dont la
/// mesure a montré qu'il était faux — aujourd'hui mika-qa seul.
pub fn max_tool_steps_for_agent(agent_id: &str, default: usize) -> usize { … }

/// Plafond de mika-qa. 30 et non 20 : voir la chaîne de justification du plan
/// de mika#2926 — prescription de 13 steps, 15 à 28 `run_shell` mesurés par tour
/// coupé, `total_calls` maximum observé de 31.
pub const MAX_TOOL_STEPS_MIKA_QA: usize = 30;
```

L'unique site de résolution est `agent_loop/mod.rs:1696`, où `max_steps` est
calculé une fois pour la boucle :

```rust
let max_steps =
    crate::planning::policy::max_tool_steps_for_agent(db.agent_id(), mode.max_steps());
```

Ce site couvre **les trois modes d'un coup** — `Conversation`, `Silent { .. }`
(donc les tours de callback, dont `max_steps` vient de
`SilentTrigger::max_steps()` en `mod.rs:7534`) et `Team` — parce que `run_loop`
est le seul endroit où le plafond est lu avant la boucle `for step in 0..max_steps`.
`db.agent_id()` y est déjà disponible : le census de mika#2877 l'appelle à
`mod.rs:5184`, dans la même fonction.

`LoopMode::max_steps()` et les trois constantes existantes ne changent pas — elles
restent la source du `default`. AC3 est tenu structurellement : un agent dont le
nom n'est pas dans la table reçoit exactement ce qu'il recevait.

### R2 — Le nombre du prompt suit le const, et une garde le tient

`skills/bundled/qa-review/system_prompt.md:17` passe de 14 à 30, et
`crates/mika-agent/tests/qa_review_executes_repo_guards.rs` — le fichier qui
porte déjà les assertions sur ce prompt, avec son helper `qa_review_prompt()`
lisant `skills/bundled/qa-review/system_prompt.md` — gagne la garde :

```rust
/// Le plafond annoncé par le prompt est celui que le moteur sert (mika#2926).
#[test]
fn mika2926_le_plafond_annonce_par_le_prompt_est_celui_du_moteur() { … }
```

Elle extrait l'entier de `You have a maximum of N tool steps per turn.` et le
compare à `max_tool_steps_for_agent("mika-qa", MAX_TOOL_STEPS)`. Deux assertions,
pas une : la phrase doit être **présente** (sinon la garde deviendrait verte en
disparaissant avec sa cible), et le nombre doit être **égal**.

### R3 — Les deux catégories sans borne en reçoivent une

Dans `qa-review/system_prompt.md`, sous « Efficiency rules », la borne sur C4 et
C5 telle que le § *resserrage* la chiffre, avec sa sortie : au-delà, le tour
émet `hold[review]` en nommant la vérification qui n'a pas pu être faite, plutôt
que de continuer à explorer. Payée par le trim nommé de la ligne 234.

Dans `qa-review-build-callback/system_prompt.md`, la borne à deux tentatives sur
la re-lecture du plan, puis `block[pipeline]` — la disposition que ce prompt
prescrit déjà pour un plan illisible.

### R4 — Le corpus rejoué d'AC2

`crates/mika-agent/tests/eval/test_qa_step_budget_2926.rs`, sur le patron de
`test_max_steps_tool_census_2877.rs` : `EvalHarness::builder()` + le capteur de
tracing de mika#2305 (`test_context_scope_observability_2305::{Captured, capture}`),
réutilisé et non réécrit, pour la même raison qu'en 2877 — son
`keep_registry_multi_dispatcher()`, sans quoi le test serait vert ou rouge selon
l'ordre des threads.

Une **seule** séquence de réponses mock, de la forme de callback-0f07c520 : 23
`run_shell`, puis `run_gh("pr review … VERDICT: hold[review]")`. Deux tours la
rejouent, et c'est la paire qui est le livrable :

- **Positif** — `.agent_id("mika-qa").callback_turn(true)` : plafond 30, le
  `pr review` du step 24 est servi. Assertions : zéro événement
  `max_steps_tool_census`, et la revue est enregistrée (`pr_reviews_posted`).
- **Contrôle négatif, la forme d'avant** — même séquence, `.agent_id("mika-dev")` :
  plafond 20, le tour est coupé au step 20. Assertions : exactement un événement
  `max_steps_tool_census` avec `max_steps = "20"` et `per_tool` portant
  `run_shell`, et **aucune** revue enregistrée.

Le discriminant entre les deux tours est l'`agent_id` et **rien d'autre** — même
mock, même séquence, même nombre de réponses. C'est ce qui fait que la paire
atteste le plafond par agent et pas un artefact de fixture.

Déclaration du module dans `crates/mika-agent/tests/eval.rs`, dans le bloc
`mod eval`.

## Non-Requirements (hors de ce vol, délibérément)

- **`qa_callback_verdict` / `no_target_stamp`.** Le ticket l'écarte comme cause et
  `read_qa_review_pr_target` décrit le cas comme nominal. Rien n'y est touché.
- **L'enveloppe de temps du tour.** Nommée comme risque, mesurée par la sonde
  d'AC4, pas modifiée. Si la sonde montre le déplacement, c'est un ticket de
  suivi avec sa propre mesure — et ce vol lui laisse l'instrument pour l'ouvrir.
- **Un outil borné qui remplace la recette 2B** (un `qa_repo_guards` prenant
  `{repo, head_ref, base_ref, number, author, labels, body}` et rendant la sortie
  des guards en un appel). C'est la suite naturelle de C1, mais elle est
  **conditionnée par la part manquante du relevé** : sans l'attribution par
  catégorie du § 2, construire un outil pour C1 serait spéculatif, alors que C1
  est déjà borné à 1 par le prompt. Tracked in: senara-solutions/mika#2926 —
  à ouvrir comme ticket de suivi après la première lecture de la requête SQL du
  § 2 sur l'hôte.
- **Le relevé par catégorie lui-même.** Inatteignable depuis le bac à sable (§ 2,
  trois faits vérifiés). La décision de ce plan n'en dépend pas : elle repose sur
  le relevé du prescripteur (§ 3) et sur la divergence prompt/moteur (§ 4), tous
  deux lisibles dans le dépôt.

## Verification Contract

| # | vérification | commande |
|---|---|---|
| V1 | AC2, la paire | `cargo test -p mika-agent --test eval qa_step_budget_2926` |
| V2 | la garde prompt↔const | `cargo test -p mika-agent --test qa_review_executes_repo_guards mika2926` |
| V3 | le plafond par agent, unités | `cargo test -p mika-agent policy::tests` |
| V4 | aucune régression sur les plafonds existants | `cargo test -p mika-agent` (dont `test_max_steps_continuation`, `test_max_steps_tool_census_2877`, `test_callback_terminal_action` — tous sur l'agent par défaut `mika`, donc inchangés) |
| V5 | le prompt reste sous son gate | `wc -c skills/bundled/qa-review/system_prompt.md` < 77 824, et `cargo test -p mika-agent --test bundled_skills_load` (il **panique**, il n'avertit pas) |
| V6 | structure des bundles | `make verify-bundled-skills` |
| V7 | lints et pipeline | `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` |

### Rouge-avant, exécutable par le relecteur

1. **V2 rougit sur l'état actuel du dépôt**, sans rien commenter : le prompt porte
   14, le const vaudra 30. Message attendu : le plafond annoncé (14) diffère de
   celui que le moteur sert à mika-qa (30). Le même commit corrige le prompt —
   c'est la seule violation existante, et elle n'atterrit pas.
2. **V1, moitié positive** : remplacer `"mika-qa"` par `"mika-dev"` dans le tour
   positif. Il rougit — *« expected a posted pr review, found none »* — et la
   moitié négative reste verte. C'est l'information qui dit que la moitié négative
   seule n'atteste rien : elle atteste une absence, et sous cette recette
   l'absence est totale.
3. **V1, moitié négative** : remplacer `"mika-dev"` par `"mika-qa"` dans le tour
   négatif. Il rougit sur `expected exactly one max_steps_tool_census event, got 0`.

## Definition of Done

- [ ] `max_tool_steps_for_agent` existe dans `planning/policy.rs` avec
      `MAX_TOOL_STEPS_MIKA_QA = 30` et ses tests unitaires (mika-qa → 30 ;
      un agent absent de la table → le `default` passé, pour les trois modes).
- [ ] `agent_loop/mod.rs:1696` est le **seul** site qui résout le plafond ; un
      `grep` de `mode.max_steps()` hors de `LoopMode` et de ses tests ne trouve
      rien d'autre.
- [ ] `qa-review/system_prompt.md` porte 30, la borne C4/C5, et le trim de la
      ligne 234 ; `wc -c` sous 77 824.
- [ ] `qa-review-build-callback/system_prompt.md` porte la borne à deux
      tentatives sur la re-lecture du plan.
- [ ] La garde R2 est verte après correction du prompt et rouge avant (recette
      ci-dessus, point 1).
- [ ] `test_qa_step_budget_2926.rs` existe, est déclaré dans `tests/eval.rs`, et
      ses deux moitiés passent — avec les deux recettes de rouge-avant tenues.
- [ ] V4 à V7 verts.
- [ ] `docs/agent-internals/observability.md` § mika#2877 gagne un renvoi : le
      plafond de mika-qa n'est plus `MAX_TOOL_STEPS`, donc le champ `max_steps`
      d'une ligne de census vaut désormais 20 **ou** 30 selon l'agent, et un
      opérateur qui groupe par ce champ doit le savoir.
- [ ] `docs/operator/environment-variables.md` : la sonde d'AC4 et ses deux
      compteurs y sont documentés (pas dans `CLAUDE.md`, qui est sous cap).
- [ ] Corps de PR : `Closes #2926`, et
      `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Transcrits du ticket, verbatim, avec la façon dont ce plan les ferme.

- **AC1.** Le plan contient le relevé : pour au moins 5 des 13 tours, la catégorie
  des commandes `run_shell` (lecture du diff, `git log`, `cargo`, recherche dans
  le dépôt…), avec leur compte.
  → § *Le relevé*, en quatre parties : les comptes mesurés de **7** tours (§ 1),
  la raison pour laquelle l'attribution par catégorie n'est pas dans le log et la
  requête qui la produit sur l'hôte (§ 2, avec les trois faits qui établissent
  qu'aucun pilote ne peut l'exécuter), le relevé **des six catégories du
  prescripteur** avec leur compte et la propriété que deux d'entre elles sont sans
  borne (§ 3), et le fait indépendant qui porte la décision (§ 4).
- **AC2.** Sur un corpus rejoué de la forme de la session callback-0f07c520 (un
  callback de build réussi sur une PR de la boucle, mika#2920 à 0a55d28a), le tour
  pose un verdict sous le budget. Le contrôle négatif est dans le même test : la
  forme d'avant épuise le budget.
  → R4. Même séquence de 23 `run_shell` + `pr review`, deux tours, l'`agent_id`
  pour seul discriminant. V1 et ses deux recettes de rouge-avant.
- **AC3.** Si la voie (a) est retenue, le nouveau plafond est nommé à un seul site
  et justifié par la mesure d'AC1. Il ne s'applique pas aux autres agents.
  → R1. Un seul site de résolution (`mod.rs:1696`), un seul const
  (`MAX_TOOL_STEPS_MIKA_QA`), la chaîne de justification au § *Le nombre*, et la
  non-application aux autres agents tenue **structurellement** : un agent absent
  de la table reçoit le `default` du mode, ce que le contrôle négatif de V1
  atteste sur mika-dev.
- **AC4, la sonde post-déploiement.** Après `make deploy`, sur les 7 jours
  suivants, le compte de `max_steps_tool_census` de mika-qa baisse depuis 13 en
  3,5 jours. Le plan donne la commande qui le compte.
  → § *Sonde post-déploiement* ci-dessous.

## Sonde post-déploiement (AC4)

Deux relevés — un au déploiement (`J0`), un à `J+7` — et la différence est le
compte de la fenêtre. **Deux compteurs, et la lecture est sur la paire.**

```sh
L=${MIKA_SPIRIT_LOG_FILE:-/var/log/mika/server.log}

# (1) le compteur du ticket : les épuisements de steps de mika-qa
grep max_steps_tool_census "$L" | jq -r 'select(.agent_id == "mika-qa") | .max_steps' | wc -l

# (2) le compteur du déplacement : les coupures par enveloppe de temps
grep -c "agent deadline exceeded" "$L"
```

La différence de (1) entre `J0` et `J+7` est le compte de la fenêtre. Prendre le
compte total à deux instants plutôt qu'un filtre par date est délibéré : le nom du
champ d'horodatage du JSON de `tracing-subscriber` n'a pas à être deviné pour que
la soustraction soit juste. La seule chose qui la casserait est une rotation du
log entre les deux relevés — relever alors `(1)` sur les fichiers tournés aussi.

**Lecture, et le seuil.** 13 en 3,5 jours fait 3,7/jour. Succès si (1) rend moins
de **13 sur 7 jours** (≤ 1,9/jour, soit une baisse du taux d'au moins la moitié)
**et** si (2) n'a pas monté de plus que ce que (1) a baissé. Le second terme est
la raison d'être de la paire : sans lui, un épuisement qui a simplement migré du
budget de steps vers l'enveloppe de 300 s se lirait comme une guérison.

**Sonde de conformité, à relever d'abord** — elle dit que l'instrument tourne, et
elle est de mika#2877 :

```sh
grep -c "agent exceeded max tool steps" "$L"   # doit égaler (1) tous agents confondus
grep -c max_steps_tool_census           "$L"
```

Les deux lignes sont émises au même site sans condition : une inégalité est un
défaut de l'instrument, pas un détail de log, et elle invalide le relevé avant
toute lecture.

## Fire-Disposition

Ce vol livre **un** détecteur : la garde R2, dont le chemin de succès est
« le nombre du prompt égale le const du moteur », c'est-à-dire « aucune
divergence trouvée ». Le test AC2 (R4) n'en est pas un — ses deux moitiés
assertent un comportement (« un verdict a été posé », « le budget a été épuisé »),
pas l'absence d'une violation dans un corpus.

**Option retenue : (a) — exception nommée en allowlist, avec une allowlist
vide, et c'est la forme exacte que l'option prend ici.**

- **Population des violations existantes : une seule**, la ligne 17 de
  `qa-review/system_prompt.md` qui porte 14. Elle est **corrigée dans le même
  commit** que la garde (ordre de commit ci-dessous, commits 2 et 3 : le const
  d'abord, puis le prompt et la garde ensemble). La garde atterrit donc **armée et
  verte**, sans aucune entrée d'allowlist — il n'y a rien à exempter.
- **Pourquoi pas (b), livrer désarmé** : il n'y a pas de résidu à trier. Un
  `#[ignore]` sur une garde qui passerait sans lui crée un suivi vide et une garde
  qu'aucun pipeline n'exécute — la classe que `suite-wiring-lint` existe pour
  refuser.
- **Pourquoi pas (c), halte-et-remontée** : la correction est de deux caractères
  et le relecteur la voit dans le diff.
- **Auto-nettoyage, par construction et non par assertion ajoutée** : la garde ne
  compare pas à une valeur gravée mais à
  `max_tool_steps_for_agent("mika-qa", MAX_TOOL_STEPS)`. Une exception ne peut pas
  devenir stale parce qu'il n'y en a aucune : le jour où le plafond bouge, la garde
  rougit sur le prompt jusqu'à ce qu'il suive. C'est précisément la dérive
  silencieuse de 14 vs 20 qu'elle rend impossible à réintroduire.
- **Portée délibérément étroite** : la garde ne couvre que `qa-review`. Les autres
  prompts bundled n'annoncent aucun plafond de steps (`grep` du § 4 : une seule
  occurrence dans tout `skills/bundled/`), donc une garde générique n'aurait, à ce
  jour, aucune population — et une garde sans population est une garde qui ne
  refuse rien.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/planning/policy.rs` | 95 |
| `crates/mika-agent/src/agent_loop/mod.rs` | 14 |
| `skills/bundled/qa-review/system_prompt.md` | 12 |
| `skills/bundled/qa-review-build-callback/system_prompt.md` | 9 |
| `crates/mika-agent/tests/qa_review_executes_repo_guards.rs` | 70 |
| tests (`crates/mika-agent/tests/eval/test_qa_step_budget_2926.rs`) | 150 |
| `crates/mika-agent/tests/eval.rs` | 1 |

Total estimé : 351 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 351 est sous la borne par vol (429)
et très loin du seuil par PR (1000). La pièce qui aurait fait sortir ce vol de sa
borne — l'outil borné `qa_repo_guards` qui remplacerait la recette 2B, ≈ 200
lignes de handler plus son test — est explicitement renvoyée aux
Non-Requirements, et pour une raison de fond et non de place : elle est
conditionnée par la part du relevé qu'aucun pilote ne peut lire. Le reste du vol
est serré par construction : les deux prompts ne bougent que de 21 lignes au total
parce que l'un est sur son plafond dur d'octets, et le gros du volume est la paire
de tests qui ferme AC2.

## Ordre de commit

1. `test(2926)` — `test_qa_step_budget_2926.rs` + sa déclaration dans `eval.rs`.
   Rouge sur les deux moitiés : le plafond de mika-qa vaut encore 20.
2. `feat(2926)` — `max_tool_steps_for_agent` + `MAX_TOOL_STEPS_MIKA_QA` dans
   `policy.rs`, et le site unique de résolution dans `agent_loop/mod.rs`.
   Le commit 1 passe au vert. V3, V4.
3. `fix(2926)` — le prompt `qa-review` (30, borne C4/C5, trim de la ligne 234),
   le prompt `qa-review-build-callback` (borne à deux tentatives), et la garde R2,
   **dans le même commit** : c'est ce qui fait atterrir le détecteur armé et vert
   sans allowlist. V2, V5, V6.
4. `docs(2926)` — `observability.md` (le champ `max_steps` vaut 20 ou 30 selon
   l'agent), `environment-variables.md` (la sonde d'AC4 et sa paire de compteurs),
   puis `scripts/sync-agent-docs.sh`.

Chaque commit laisse l'arbre compilable ; seul le 1 laisse un test rouge, qui est
son objet.
