# mika#2532 — Le stderr d'un handler long-running n'est plus jeté, et le message de crash nomme son étape

**Ticket :** senara-solutions/mika#2532
**Type :** fix (p1 — observabilité)
**Date :** 2026-09-25

---

## 1. Mesure fondatrice

Le handler `skills/bundled/build-mika/handlers/run.sh` a crashé **4 fois** sur la QA
de PR #2530, chaque tentative, avec le même message dans `tasks.result` :

> `HANDLER CRASH (exit code 1). Script failed before building result.`

Instances (2026-09-25) : 12:52:02 (`5624ea27`), 13:05:26 (`4a877600`), 13:48:20
(`5078ef8e`), 14:02:57 (`03a16846`). Même chemin, même PR, chaque QA — **classe,
pas transitoire**. Conséquence : la vérification de build ne confirme rien, QA rend
`COMMENTED` au lieu d'`APPROVED`, et #2530 reste hors merge avec une CI verte.

---

## 2. Ce que la lecture du code déplace — et c'est le premier livrable

Le ticket liste quatre candidats et conclut que la cause est **inobservable**. La
lecture du substrat en réfute trois, en ferme un quatrième, et corrige la mécanique
du défaut p1. Ces six constats sont le socle du reste du plan.

### F1 — Le crash est à la ligne 65, `cd "$CWD"`, et c'est le **seul** site possible

Le message « HANDLER CRASH » est écrit **uniquement** par `deliver_callback`, installé
en trap EXIT à la l.56. Le crash est donc **postérieur** à la l.56. Entre la l.56 et la
première assignation de `RESULT` (l.73/77), il y a exactement quatre instructions :

| ligne | instruction | peut-elle produire exit 1 ? |
|---|---|---|
| 59 | `if [ -z "$CWD" ]; then` | non — une condition de `if` n'est jamais fatale sous `set -e`, et un `if` sans `else` dont la condition est fausse rend 0 |
| 60 | `_DEFAULT="…"` | non — assignation littérale |
| 61 | `CWD=$(cd … && pwd -P) \|\| CWD="$_DEFAULT"` | non — protégée par `\|\|` |
| **65** | `cd "$CWD" \|\| { echo … >&2; exit 1; }` | **oui — `exit 1` explicite** |

L'exit code observé est **exactement 1**, celui du littéral de la l.65. La l.67
(`cargo build`) est sous `set +e` et assigne `RESULT` dans les deux branches, donc
ne peut pas produire ce message — ce que le ticket avait déjà établi.

**La cause est donc localisée à la ligne près, par élimination structurelle, sans
stderr.** C'est une rectification de la prémisse du ticket (« on ne peut pas dire
quelle ligne a échoué ») : on le peut. Ce qu'on ne peut pas dire, c'est *pourquoi*
ce `cd` a échoué — voir F5.

### F2 — Trois des quatre candidats sont réfutés, et leur mode de panne est **pire**

`command -v jq` (l.15), `command -v mika` (l.16) et le parse jq (l.25-27) sont
**avant** l'installation du trap. Un échec là ne produit **aucun** callback : la
tâche resterait en vol jusqu'à son timeout, sans jamais écrire `tasks.result`. Le
message observé **prouve** que le trap était installé.

Corollaire à ne pas perdre : ces trois sites ont un mode de panne **strictement
pire** que celui qui a été mesuré — le silence complet. Le ticket ne les a pas
nommés comme tels parce qu'ils ne se sont pas produits. Le plan les referme (L3).

### F3 — Le stderr n'est pas « non capturé » : il est capturé, puis **jeté**

C'est la mécanique exacte du défaut p1, et elle n'est pas celle que le ticket
suppose. `spawn_long_running_exec` (`crates/mika-agent/src/skills/executor.rs`)
pose `stderr(Stdio::piped())`, et sur `!status.success()` il **lit réellement** le
stderr (capé à `MAX_OUTPUT_LEN = 10_000`), compose
`err_msg = "Process Exit code: 1: <stderr>"`, puis appelle
`db.update_task_failed(&task_id, &err_msg)`.

Mais le trap du handler a **déjà** livré le callback (`mika ask --task-complete`),
donc la tâche est **terminale**. `update_task_failed` rend alors `Ok(false)` et
n'écrit rien. Et aucune des deux branches ne journalise `err_msg` :

- `Ok(true)` → `warn!(task_id, %code_display, "long-running exec failed")` — porte
  le code, **pas** le stderr ;
- `Ok(false)` → `info!(task_id, %code_display, "… already in terminal state")` —
  idem.

**`stderr_text` est donc lu en mémoire puis abandonné sans atteindre aucune surface.**
Le défaut n'est pas « ajouter une capture », c'est « ne pas perdre celle qui existe ».

Et cette course est **la norme, pas l'exception** : tout handler long-running livre
son résultat par callback — c'est le contrat (`tools.json` `long_running: true`).
Donc **pour tout handler long-running qui sort non-zéro après avoir livré son
callback, le stderr est structurellement invisible**. Quatre handlers de dispatch
sont concernés (`build-mika`, `deploy-mika`, `resolve-pr-conflicts`,
`address-pr-comments`), plus `dev-pilot` / `dev-groom` via `dispatch-lib.sh`.

### F4 — `MIKA_PLATFORM_DIR` ne peut **jamais** atteindre ces handlers

`spawn_long_running_exec` appelle `sandboxed_pilot_env`, dont
`is_sandbox_env_allowed` retourne `false` pour **tout** `MIKA_*` — c'est le premier
test de la fonction, renforcé par un `debug_assert`. Donc la l.60
`${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}` prend **toujours** la branche
de droite.

C'est une **branche morte** : elle donne au handler l'apparence de respecter un
réglage d'opérateur qu'il ne peut pas lire. Le défaut est donc toujours
`$HOME/workspace/mika-platform/mika` — un chemin dont rien ne garantit l'existence
et qui n'est **jamais** celui du worktree d'une PR.

La classe compte **six sites** sur quatre handlers :

| fichier | lignes |
|---|---|
| `build-mika/handlers/run.sh` | 60 |
| `deploy-mika/handlers/run.sh` | 36, 68, 69 |
| `resolve-pr-conflicts/handlers/run.sh` | 112, 113 |
| `address-pr-comments/handlers/run.sh` | 77, 78 |

### F5 — Le prompt prescrit au modèle une variable que **rien** ne développe

`qa-review/system_prompt.md:568` et `qa-review-build-callback/system_prompt.md:30`
prescrivent littéralement :

```
worktree = $MIKA_PLATFORM_DIR/.claude/worktrees/${sanitized_branch}/mika/
```

puis Step 3e.3 : `build_mika(cwd=<worktree>)`. `build-mika/system_prompt.md:20-21`
répète la même forme.

Or `MIKA_PLATFORM_DIR` est scrubbé **des deux côtés** :

- dans le handler (allowlist positive de `sandboxed_pilot_env`, F4) ;
- dans `run_shell` — `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh:28`
  fait `for _mika_var in $(env | grep '^MIKA_' …); do unset …; done` **avant**
  d'exécuter la commande du modèle (il la lit l.22 uniquement pour la passer en
  *argument* à la garde mika#2449, jamais dans l'environnement de la commande).

Si le modèle recopie la forme littérale dans l'argument JSON, `CWD` vaut
`$MIKA_PLATFORM_DIR/.claude/…`, `cd` échoue **à coup sûr** — signature F1,
déterministe, rejouée à chaque QA. C'est l'hypothèse la plus économique pour
expliquer « 4 fois, même PR, chaque tentative ».

**Elle n'est pas tranchée**, et le plan ne prétend pas la trancher : la valeur
réellement passée vit dans `tool_calls`, table que le bac à sable de dispatch ne
monte pas. C'est précisément pourquoi L1 (l'observabilité) est livré **avant** toute
conclusion sur la valeur.

### F6 — L'état corrélé du ticket ne réfute rien, et la mesure n'est plus rejouable

Le ticket note que le worktree existe et que `cd` y réussit. Ce test a été fait
depuis un shell d'opérateur, avec un chemin **déjà développé** — il ne dit rien de la
chaîne que le modèle a réellement passée au handler. Et aujourd'hui ce worktree
n'existe plus (PR #2530 mergée, `b1d96948` sur `main`, worktree fauché) : la mesure
d'origine est **perdue**. Une raison de plus pour que ce plan livre l'observabilité
d'abord et la conclusion ensuite.

---

## 3. Requirements

| # | Exigence | AC couvert |
|---|---|---|
| R1 | Le stderr d'un handler long-running en échec atteint une surface lisible **par task**, y compris — et surtout — quand la tâche est déjà terminale | AC1 |
| R2 | Le message de crash pré-résultat nomme **l'étape** qui a échoué et son détail (cwd, dépendance, parse) | AC3 |
| R3 | Le trap de livraison est installé aussitôt que `TASK_ID` est connu, pour qu'aucune étape postérieure ne puisse échouer en silence | AC3 |
| R4 | Les branches mortes `${MIKA_*:-…}` des handlers long-running sont supprimées ; le chemin de plateforme devient réellement configurable par le canal qui traverse | AC2 |
| R5 | Les prompts cessent de prescrire au modèle une variable qu'aucun des deux environnements ne développe | AC2 |
| R6 | Un `cwd` incomposable (non absolu, variable littérale, inexistant, pas un répertoire) est **refusé en le nommant**, jamais laissé échouer sous `cd` | AC3, AC4 |
| R7 | Les détecteurs sont livrés armés, avec contrôle négatif vu rouge et allowlist vide | AC4 |

---

## 4. Design

### L1 (R1) — Le stderr n'est plus jeté

Dans `spawn_long_running_exec`, branche `!status.success()` : émettre
**inconditionnellement** — que `update_task_failed` ait écrit ou non — un `warn!`
nommé portant `task_id`, `code_display` et `stderr_text` (déjà capé à 10 000 octets
par l'exécuteur), plus une ligne `audit_events` pour la lecture SQL.

Le nom est neuf et **SOLE WRITER** : `long_running_exec_stderr`.

**Divergence assumée avec la lettre de l'AC1** (« per-dispatch, comme le sink
forensique de Signal S »), et sa justification :

- Le stderr est **déjà borné à 10 Ko** par l'exécuteur : il n'y a pas de volume à
  externaliser dans un fichier.
- Un sink fichier neuf demande un répertoire, une clé de configuration, une
  rétention et un nettoyage — et surtout il crée une surface **dont l'absence se lit
  comme un silence**. C'est la panne que ce dépôt a mesurée deux fois sur exactement
  cette forme (Signal M et Signal Q, mika#2050 : un sink documenté qui ne reçoit
  rien, publié sous un régime « zéro attendu », donc illisible).
- Le `.stderr` de Signal S existe parce que `dispatch-lib.sh` **redirige lui-même**
  son stderr (`2>"$STDERR_FILE"`) : il n'y avait pas d'autre porte. Ici l'exécuteur
  Rust tient déjà le texte en main et écrit déjà dans le journal que l'opérateur
  grep par `task_id`.
- Résultat : `grep <task_id> "$MIKA_SPIRIT_LOG_FILE"` rend le stderr,
  `mika tasks get <id>` rend le RESULT. **Deux surfaces déjà connues, aucune neuve.**

**Portée** : ce correctif est **général** — il couvre les six handlers long-running,
pas seulement `build-mika`. C'est voulu et c'est dit : le défaut est dans
l'exécuteur, pas dans `build-mika`.

### L2 (R2, R6) — Le message de crash nomme son étape

Une variable `STAGE` mise à jour au fil du handler (`deps`, `parse`, `resolve_cwd`,
`enter_cwd`, `build`, `deliver`). `deliver_callback` compose :

```
HANDLER CRASH (exit code N) at stage '<STAGE>'. <détail>
```

Le détail est spécifique à l'étape ; pour `enter_cwd` il cite le cwd **verbatim** et
sa **provenance** (argument `cwd` de l'outil vs. défaut résolu). Le RESULT devient
auto-diagnostique sans sortir de `tasks.result`, la surface que l'opérateur lit en
premier.

**Le RESULT du callback n'est jamais réécrit depuis l'extérieur** : `qa_build_callback`
le lit pour reprendre la revue, et le modifier casserait la reprise. C'est le handler
lui-même qui compose son message — aucun conflit avec L1, qui écrit ailleurs.

**R6, validation de `CWD` avant le `cd`** — quatre refus nommés, chacun avec son mot
dans le RESULT : chemin non absolu, chemin contenant un `$` littéral (la faute F5),
chemin inexistant, chemin qui n'est pas un répertoire. Un `cd` qui échoue est une
information pauvre ; un refus qui nomme *laquelle* des quatre fautes a eu lieu est
ce que l'AC3 demande.

### L3 (R3) — Le trap est installé dès que `TASK_ID` est connu

Réordonner : lire l'entrée → vérifier `jq` → parser `TASK_ID` → **installer le trap**
→ tout le reste (`command -v mika`, scrub, résolution du cwd, build). Après ce point,
tout échec produit un callback nommant son étape.

**Irréductible nommé plutôt que caché** : `jq` absent empêche de parser `TASK_ID`,
donc aucun callback n'est possible ; `mika` absent empêche de le livrer. Ces deux
étapes restent silencieuses **par construction** — et c'est très exactement L1 qui les
rend lisibles, puisque leur stderr atteint désormais le journal. Les deux moitiés se
complètent : L3 referme ce qui est refermable, L1 couvre le reste.

### L4 (R4, R5) — Le chemin de plateforme traverse, ou n'est pas prétendu

**L4a — le relais, motif maison.** `spawn_long_running_exec` injecte `PLATFORM_DIR`
(non préfixé) **après** `sandboxed_pilot_env`, exactement comme le dépôt le fait déjà
cinq fois pour cette classe (`inject_pilot_transcript_env`,
`inject_dispatch_worktree_env`, `inject_rescue_verify_env`, `inject_arch_ask_retry_env`,
`inject_pilot_dispatch_env`). La leçon est écrite au mot près dans CLAUDE.md
(mika#2508) : *nommer une variable `PILOT_*` ou `MIKA_*` ne la fait pas traverser ; le
relais explicite si.* Le précédent est déjà dans ce dépôt pour **cette variable
exacte** — `shell-exec/handlers/run.sh` la lit avant son scrub et la passe en argument.

Les six sites deviennent `${PLATFORM_DIR:-$HOME/workspace/mika-platform}`. Trois d'entre
eux nomment déjà leur variable locale `PLATFORM_DIR`, donc l'expression s'auto-référence
avec défaut — correct, et à signaler dans le commit pour éviter la confusion à la
relecture.

**Refus raisonné** : ajouter `MIKA_PLATFORM_DIR` à `SANDBOX_ENV_CORE_ALLOWLIST`. Ce
serait percer une garde anti-fuite de secret pour un confort de chemin, contre un
`debug_assert` explicite qui existe pour empêcher précisément ce geste. Le relais
obtient le même résultat sans toucher la garde.

**Pourquoi corriger les six sites et pas le seul de `build-mika`** : c'est la même
ligne, le même défaut, le même correctif d'une ligne. Laisser trois handlers avec leur
branche morte tout en livrant le scan T4 forcerait une allowlist de trois entrées —
c'est-à-dire déposer trois infractions dans un emplacement neuf, ce que la doctrine
mika#2201 refuse explicitement (« on déclare, on n'allowliste pas »).

**L4b — les prompts.** Corriger les trois prescriptions qui composent un `cwd` à partir
de `$MIKA_PLATFORM_DIR` (`qa-review:568`, `qa-review-build-callback:30`,
`build-mika/system_prompt.md:20-21`) pour qu'elles n'exigent plus une variable que rien
ne développe.

Et la moitié qui tient n'est pas celle-là : par
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, un correctif de
prompt seul ne tient pas au substrat de la boucle. **La moitié structurelle est R6** :
le handler refuse un `cwd` portant un `$` littéral et le nomme. Le prompt exprime
l'intention, le refus la tient.

### L5 (R7) — Les détecteurs

- **T1 — test shell** (`scripts/test-build-mika-handler.sh`, cible Makefile, job CI) :
  le handler avec un `cwd` inexistant, un `cwd` relatif et un `cwd` à variable
  littérale produit un RESULT nommant l'étape **et** le cwd. **Contrôle négatif vu
  rouge** : une fixture du handler d'avant rend le message générique et le test rougit.
- **T2 — test Rust** : le stderr d'un long-running en échec atteint `warn!` + audit
  **même quand la tâche est déjà terminale** (la branche `Ok(false)`, c'est-à-dire la
  population exacte du défaut). Contrôle négatif : sans l'émission, le test rougit.
- **T3 — scan SOLE WRITER** de `long_running_exec_stderr` (motif
  `canonical_tokens::tests::mika2242_the_two_audit_names_have_a_single_writer`),
  avec son pendant auto-nettoyant asserant l'allowlist vide.
- **T4 — scan de classe** : aucun handler exécuté sous `sandboxed_pilot_env` ne lit
  un `${MIKA_*:-…}`. C'est la classe F4 — une variable qui ne peut pas traverser, lue
  comme si elle pouvait. Un test comportemental ne peut pas la voir : la branche morte
  ne rend **aucune décision fausse**, elle rend seulement un réglage inopérant en
  silence.

---

## Fire-Disposition

Ce plan livre quatre détecteurs (T1–T4). Option retenue : **(a) exception nommée en
allowlist — et les quatre allowlists sont livrées VIDES**, ce qui est vérifiable plutôt
que souhaité.

| détecteur | infractions préexistantes | allowlist | justification |
|---|---|---|---|
| T1 | aucune possible — il teste le comportement neuf | néant | un test comportemental n'a pas d'allowlist ; son contrôle négatif tient sa valeur |
| T2 | aucune possible — idem | néant | idem |
| T3 | **zéro, et c'est établi** : `long_running_exec_stderr` est un nom **neuf**, donc aucun second écrivain ne peut exister à la livraison | `SOLE_WRITER_EXCEPTIONS` livrée vide + test `…_allowlist_is_empty` | quand le scan tire, on **retire** le second écrivain (doctrine mika#2201) — une exception rendrait le `GROUP BY` de l'opérateur silencieusement faux, strictement pire que le silence qu'il remplace |
| T4 | **six, toutes corrigées par L4a dans le même commit** | livrée vide + test auto-nettoyant | c'est la décision centrale de la disposition : corriger les six plutôt que d'en allowlister trois. Une allowlist née avec trois entrées est un emplacement où déposer la quatrième |

**Conduite quand T4 tire** : on route le nouveau site vers le relais `PLATFORM_DIR`,
on n'ajoute pas de ligne à l'allowlist. Un handler qui a besoin d'un `MIKA_*` a besoin
d'un relais, pas d'une dérogation.

**Assertion auto-nettoyante** : les tests `…_allowlist_is_empty` de T3 et T4 rougissent
dès qu'une entrée y apparaît, et le message nomme la conduite. Aucune exception ne peut
devenir silencieusement permanente.

**Aucun détecteur n'est livré désarmé** : la population de T4 est ramenée à zéro par
L4a dans le même commit, donc l'option (b) n'a pas de justification, et l'option (c)
n'a rien à faire remonter — la mesure est complète.

---

## 5. Verification Contract

| # | Vérification | Nature | Comment |
|---|---|---|---|
| V1 | Un `cwd` inexistant → RESULT nomme l'étape `enter_cwd` et cite le cwd | comportemental | `make test-build-mika-handler` (T1) |
| V2 | Un `cwd` portant un `$` littéral → RESULT nomme cette faute-là, distincte d'un chemin absent | comportemental | T1 |
| V3 | Contrôle négatif : la fixture du handler d'avant fait **rougir** T1 | négatif, **à voir rouge** | T1 |
| V4 | Le stderr atteint le journal et `audit_events` sur la branche `Ok(false)` | comportemental | `cargo test -p mika-agent` (T2) |
| V5 | Contrôle négatif de T2 : sans l'émission, le test rougit | négatif, **à voir rouge** | T2 |
| V6 | `long_running_exec_stderr` a un écrivain unique ; l'allowlist est vide | structurel | T3 |
| V7 | Aucun `${MIKA_*:-…}` dans un handler sandboxé ; l'allowlist est vide | structurel | T4 |
| V8 | Les six sites lisent `PLATFORM_DIR` et le relais est posé après `sandboxed_pilot_env` | structurel | T4 + revue du diff |
| V9 | Les trois prompts ne prescrivent plus `$MIKA_PLATFORM_DIR` dans un `cwd` | structurel | `grep` sur le diff |
| V10 | `make verify-bundled-skills` passe (invariants structurels des bundles) | structurel | cible existante |
| V11 | `cargo clippy` et `cargo fmt --check` propres | structurel | CI |

**Ce qui n'est PAS testable ici, écrit plutôt que découvert** : « le build_mika ne
crashe plus sur une QA nominale » (AC4, premier volet) s'exécute contre une PR réelle,
avec mika-spirit déployé et un worktree vivant. Le bac à sable de dispatch ne monte ni
la base ni les worktrees des autres PR. C'est la sonde S1 ci-dessous, un geste
d'opérateur — pas une assertion que ce plan peut porter.

---

## 6. Sondes post-déploiement, et leurs quatre haltes

> **Préalable à toute sonde.** `skills/bundled/` est une projection du **binaire**, pas
> du checkout (mika#2340). Un handler édité dans l'arbre est invisible tant que
> `make deploy` n'a pas reconstruit puis seedé. Vérifier d'abord :
> `cat ~/.mika/skills/.manifest-writer` doit porter le sha qu'on vient de bâtir.

### S1 — AC4, premier volet (première QA de PR après déploiement)

Le callback `build_mika` rend un RESULT « Build succeeded » ou « Build FAILED », jamais
« HANDLER CRASH ».

**Halte 1 — il crashe encore.** Ne pas retoucher le handler par réflexe : lire d'abord
le RESULT, qui nomme désormais son étape (L2), puis le stderr :

```bash
grep long_running_exec_stderr "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{task_id, code_display, stderr}'
```

L'étape nommée dit quel remède s'applique, et les quatre ne sont pas le même.

### S2 — AC1, contrôle **positif** (48 h)

```bash
# La surface reçoit-elle quelque chose ?
grep long_running_exec_stderr "$MIKA_SPIRIT_LOG_FILE" | wc -l
```

```sql
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'long_running_exec_stderr' GROUP BY 1 ORDER BY 2 DESC;
```

**Régime attendu : non vide et faible.** Chaque ligne est un handler long-running sorti
non-zéro dont le stderr était jusqu'ici perdu.

**Halte 2 — zéro ligne.** On ne peut **rien** conclure. Un zéro est compatible avec
« aucun handler n'a échoué » (sain) **et** avec « l'émission n'est pas déployée »
(classe mika#2340). Établir le déploiement avant toute conclusion : *une garde qu'on
n'a pas déployée se lit exactement comme une flotte saine* (mika#2205). C'est
littéralement la forme de panne que ce ticket ferme, reproduite un cran plus loin.

### S3 — Attribution (30 jours)

Le `GROUP BY` ci-dessus donne la répartition par task. Un handler qui domine largement
est un défaut **amont** à traiter à sa source, pas un seuil à régler ici.

**Halte 3 — le compte porte du trafic nominal** (plusieurs par heure). Ce n'est pas
que la surface est trop large : c'est qu'une population de handlers sort non-zéro après
avoir livré son callback, ce qui était **invisible** jusqu'ici. C'est un **résultat**, à
écrire comme tel, et il ouvre son propre ticket avec ce compte en précondition.

### S4 — Contrôle négatif de bruit (7 jours)

Aucune ligne `long_running_exec_stderr` sur un dispatch qui a abouti proprement.

**Halte 4 — une occurrence sur un dispatch sain.** L'émission est hors de la branche
`!status.success()` : corriger le placement, **pas** filtrer en aval.

---

## 7. Definition of Done

- [ ] `spawn_long_running_exec` émet `long_running_exec_stderr` (WARN + `audit_events`)
      inconditionnellement dans la branche `!status.success()`, avec `task_id`,
      `code_display` et le stderr capé.
- [ ] `build-mika/handlers/run.sh` : trap installé dès que `TASK_ID` est connu ;
      variable `STAGE` ; message de crash nommant l'étape et son détail ; validation de
      `CWD` avec quatre refus nommés.
- [ ] Relais `PLATFORM_DIR` injecté après `sandboxed_pilot_env` ; les six sites des
      quatre handlers le lisent ; aucune branche `${MIKA_*:-…}` ne subsiste.
- [ ] Les trois prompts ne prescrivent plus `$MIKA_PLATFORM_DIR` dans un `cwd`.
- [ ] T1–T4 livrés armés, allowlists vides, contrôles négatifs **vus rouges**.
- [ ] Cible `make test-build-mika-handler` + job CI sur le modèle de
      `pilot-push-lint` / `shared-checkout-guard-lint`, avec l'étape « Pin the guard's
      negative behaviour » (discipline mika#2103).
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
      `make verify-bundled-skills` propres.
- [ ] `CLAUDE.md` : entrée nommant la surface `long_running_exec_stderr`, son régime
      attendu, ses quatre haltes, et le relais `PLATFORM_DIR` à côté de la doctrine
      mika#2508.

---

## 8. Acceptance criteria

Transcrites du ticket senara-solutions/mika#2532.

1. **Persister le stderr du handler build-mika** vers une surface lisible par task
   (per-dispatch, comme le sink forensique de Signal S), pour que la cause d'un crash
   pré-résultat soit diagnosticable — sans quoi tout AC de correction est non
   vérifiable.
   → **L1**, avec la divergence de surface argumentée au § 4 (journal + `audit_events`
   grep-ables par `task_id`, plutôt qu'un sink fichier neuf dont l'absence se lirait
   comme un silence — classe mika#2050). Vérifié par V4, V5, V6 ; mesuré par S2.

2. Sur la base du stderr rendu lisible, **identifier et corriger la cause exacte** du
   crash pré-résultat sur cette classe de PR.
   → **L4** referme les deux causes structurelles établies **par lecture** (F4 branche
   morte, F5 variable non développable). La cause est par ailleurs localisée à la ligne
   près (F1). Le volet qui reste conditionné à une mesure — la valeur exacte de `CWD`
   passée par le modèle — est nommé comme tel : il vit dans `tool_calls`, hors du bac à
   sable, et S1 le tranche. Vérifié par V8, V9 ; mesuré par S1.

3. Le message de crash générique doit nommer *quelle* étape a échoué (cwd introuvable /
   jq absent / mika absent / parse), pas seulement « script failed before building
   result ».
   → **L2** (variable `STAGE` + détail par étape) et **L3** (le trap couvre désormais
   `mika absent`, qui était hors de sa portée). Les deux étapes irréductiblement
   silencieuses (`jq` absent avant le parse, `mika` absent à la livraison) sont nommées
   et couvertes par L1. Vérifié par V1, V2, V3.

4. Sonde : le build_mika ne crashe plus pré-résultat sur une QA de PR nominale ; test
   négatif sur un cwd inaccessible → RESULT nomme le cwd.
   → Premier volet : **S1**, geste d'opérateur post-déploiement (non exécutable depuis
   le bac à sable, § 5). Second volet : **V1/V2** dans T1, avec son contrôle négatif
   V3 vu rouge.

---

## 9. Hors périmètre, délibérément

- **`$MIKA_PLATFORM_DIR` dans les commandes `run_shell` du prompt qa-review**
  (l.9, 12, 205, 279). Même variable, même scrub (F5), mais population et remède
  différents : ces lignes composent des commandes de lecture, pas un `cwd` d'outil
  long-running, et rien dans la mesure de mika#2532 ne les incrimine. **Ticket de
  suivi**, précondition : une mesure montrant qu'un `run_shell` de qa-review a
  réellement échoué sur un chemin vide. Le noter ici évite qu'il soit redécouvert.
- **Rendre le chemin de plateforme configurable par l'entrée JSON de l'outil** plutôt
  que par l'environnement. Plus propre en principe (c'est le canal que l'outil a déjà),
  mais ça change le schéma de quatre outils pour un besoin que personne n'a mesuré. Le
  relais L4a rétablit la capacité annoncée sans toucher aux schémas.
- **La fragilité `set -e` dans `deliver_callback`** (`[ … ] && return` en tête de
  fonction : la liste rend non-zéro quand la condition est fausse). Le fait observé —
  le callback arrive — prouve empiriquement que le shell en service ne tue pas le trap
  là. Durcir sans population mesurée serait un changement de comportement du chemin de
  livraison sur une intuition. **Nommé, non corrigé.**
- **Le `.stderr` des chemins `dev-pilot` / `dev-groom`**, qui ont déjà leur sink
  forensique (Signal S) et leurs deux limites connues (le chemin revise ne persiste
  rien ; Signal M écrit dans un tuyau non lu). L1 les couvre en plus, il ne les
  remplace pas.
- **Le verdict `COMMENTED` de QA** : conséquence du crash, pas sa cause. Refermer le
  crash referme le symptôme ; ajouter un filet côté verdict serait traiter l'ombre.

---

## 10. Ce que ce travail n'achète PAS

Il ne fait pas réussir un build qui échoue : il fait qu'un handler qui meurt **le
dise**. Et il n'invente aucune donnée sur les quatre crashs du 2026-09-25 — leur
worktree est fauché et leur `tool_calls` hors d'atteinte du bac à sable ; le plan
ferme deux causes structurelles établies par lecture et rend la **prochaine**
occurrence lisible. Enfin, la surface livrée devient lisible, elle ne devient pas
surveillée : **son silence ne prouve rien tant que personne n'exécute S2**, et c'est
la halte 2 qui existe pour l'empêcher d'être lu comme une bonne nouvelle.
