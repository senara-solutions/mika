# mika#2532 — Le stderr d'un handler long-running en échec survit à une tâche terminale

- **Ticket :** senara-solutions/mika#2532 (p1, re-scopé voie b — cœur observabilité R1-R3)
- **Branche :** `fix/2532/persist-long-running-handler-stderr`
- **Supersede :** plan `ebd0e46f` (branche `fix/2532/build-mika-handler-crashes-pre-result-on`, PR #2535 fermée), sur-scopé. La cause-racine `MIKA_PLATFORM_DIR` et les détecteurs cwd (R4-R7) vivent dans l'enfant mika#2536.

## La mesure

`skills/bundled/build-mika/handlers/run.sh` a crashé **4 fois sur 4** pendant la QA de PR #2530 (2026-09-25, `5624ea27` 12:52:02, `4a877600` 13:05:26, `5078ef8e` 13:48:20, `03a16846` 14:02:57), chaque tentative rendant le même message dans `tasks.result` :

> `HANDLER CRASH (exit code 1). Script failed before building result.`

Conséquence : la vérification de build ne confirme rien, la QA rend **COMMENTED** au lieu d'approuver, et un impl reste hors merge alors que sa CI GitHub est verte.

## Ce que la lecture du code déplace — premier livrable

Trois rectifications, chacune change le remède.

### R-1 — Le stderr n'est pas perdu : il est lu, formaté, puis **jeté**

`skills/executor.rs::spawn_long_running_exec` lit bien le stderr du handler (`if !status.success()`, `stderr_handle`, cap `MAX_OUTPUT_LEN`), construit `err_msg = "Process {code_display}: {stderr}"` — puis le confie à `db.update_task_failed`, dont l'`UPDATE` porte :

```sql
WHERE id = ?2 AND agent_id = ?3
  AND status NOT IN ('completed','failed','cancelled','expired','delivered')
```

Le trap du handler a **déjà** livré son callback avant de sortir, donc la tâche est `completed`. `update_task_failed` rend `Ok(false)`, la branche journalise `info!("… but task already in terminal state")`, et `err_msg` — qui contient la cause exacte — est abandonné sur place.

**Le défaut n'est donc pas une absence de capture. C'est une capture dont la seule destination refuse les rows terminales.** C'est très exactement ce que l'AC1 nomme par « y compris quand la tâche est déjà terminale ».

### R-2 — Trois cas, et un seul est cassé

| cas | trap atteint | tâche terminale | code de sortie | stderr aujourd'hui |
|---|---|---|---|---|
| **A** — crash avant le trap (`jq`/`mika` absent, pas de `TASK_ID`) | non | non | ≠ 0 | **atteint `tasks.result`** (`Ok(true)`) |
| **B** — crash après le trap, avant `RESULT` (le `cd`) | oui | oui | ≠ 0 | **jeté** (`Ok(false)`) — *le défaut mesuré* |
| **C** — succès | oui | oui | 0 | branche non prise |

Le cas A est **déjà** diagnosticable : un `jq` manquant écrit dans `tasks.result`. Ce qui n'a jamais eu de surface est le cas B — le seul où un trap qui fait correctement son travail rend la row terminale et ferme la porte derrière lui.

### R-3 — La classe est de cinq sites, et aucune machinerie shell existante ne la couvre

Cinq sites portent le trap `HANDLER CRASH` : `build-mika`, `deploy-mika`, `address-pr-comments`, `resolve-pr-conflicts`, `_shared/dispatch-lib.sh`. Les trois derniers capturent un `STDERR_FILE` et l'annexent au `RESULT` — ce qui **ressemble** à une solution et n'en est pas une ici :

```sh
STDERR_FILE=$(mktemp)                                    # resolve-pr-conflicts:303
PILOT_OUTPUT=$(claude-pilot … 2>"$STDERR_FILE")          # resolve-pr-conflicts:307
```

Ce fichier ne reçoit que le stderr du **sous-processus** `claude-pilot`. Le stderr du handler lui-même — `echo "ERROR: could not cd to $CWD" >&2`, c'est-à-dire la ligne exacte du défaut mesuré — part sur le fd 2 hérité, donc dans le `Stdio::piped()` de l'exécuteur, donc dans le cas B. Il est en outre créé 200 lignes **après** le trap : un crash avant ce point trouve `STDERR_FILE=""`.

**Les cinq sites partagent donc le même trou, et le remède qui les couvre tous est côté moteur.** C'est la moitié structurelle au sens de `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` : le moteur possède le fait, et aucun handler écrit demain ne peut se le voir retirer.

## Décisions

### D1 — La persistance est en Rust, un seul site, **sans condition sur le statut**

Dans la branche `if !status.success()` de `spawn_long_running_exec`, **avant** l'appel à `update_task_failed`, écrire le stderr sur la row.

Conditionner sur `Ok(false)` serait le réflexe et il est refusé : cela ferait dépendre l'observabilité d'une écriture concurrente, alors que « qu'a écrit ce processus sur son fd 2 » n'a rien à voir avec le statut de la row. Écrire inconditionnellement donne un chemin unique, aucune course, aucune branche à oublier — et satisfait l'AC1 à la lettre. Sur le cas A la redondance avec `tasks.result` est bénigne et la copie en metadata est, elle, scrubbée.

### D2 — La surface est `tasks.metadata`, jamais `tasks.result`

`set_task_metadata_field` ne porte **aucun** filtre de statut (`WHERE id = ?3`), et « `completed` est terminal — le statut ne transitionne plus, la metadata s'écrit encore » est un contrat explicite depuis #617. C'est la propriété que l'AC1 demande.

`tasks.result` est refusé : sur le cas B il porte le message que le tour de callback consomme et que `extract_callback_fields` / `parse_verdict` lisent. L'écraser casserait le callback ; l'annexer changerait un format de fil.

Un fichier `.stderr` par task est refusé aussi : la maison a déjà payé trois fois le prix d'un sink documenté que rien n'alimente (Signaux M, Q, S). Une surface, en base, lisible par `mika tasks get`.

### D3 — Un objet, un écrivain, un `UPDATE`

Méthode dédiée `Database::set_task_handler_failure(task_id, exit_display, stderr)` (`db/tasks.rs`) + son miroir async, écrivant l'objet `$.handler_failure` en **un seul** `json_set`. Précédent direct : `write_task_dispatch_rejection` (#1108), même besoin — « écrire une raison sur la row sans changer le statut ».

Deux `set_task_metadata_field` successifs sont refusés : non atomiques, ils laisseraient un stderr sans son code de sortie.

```json
{ "handler_failure": { "exit": "Exit code: 1", "stderr": "…", "captured_at": "2026-09-25T14:02:57Z" } }
```

`stderr` est **omis** quand le processus n'a rien écrit — un lecteur qui ne la trouve pas sait que le fd 2 est resté muet, et trouve toujours `exit` (doctrine mika#2331 : une absence n'est pas une valeur nulle déguisée).

**Fire-and-forget**, comme les quatre stamps voisins de cette fonction : `json_set` lève une erreur dure — pas un NULL — sur une `metadata` qui n'est pas du JSON valide (doctrine mika#2179), et une observabilité ne doit pas pouvoir casser la livraison qu'elle observe.

### D4 — Le cap est `MAX_OUTPUT_LEN`, celui qui existe déjà

10 000 octets, la constante que `truncate_output` applique déjà à `err_msg` et que le `tail -c 10000` du shell reprend. Un seul chiffre dans toute la maison pour cette chose. Le contenu passe par `secret_scrubber::scrub_secrets` **puis** `truncate_output` (UTF-8 sûr via `safe_truncate`).

### D5 — Le préfixe `HANDLER CRASH` est conservé

`skills/bundled/self-dev-callback/system_prompt.md:162` documente `HANDLER CRASH:` comme discriminant, et `dispatch-lib.sh:3625` le grep. L'étape s'**ajoute** au message ; elle ne le remplace pas :

```
HANDLER CRASH (exit code 1) at step 'chdir': could not cd to /nope/mika
```

### D6 — Pas de ligne `audit_events`, et la raison est écrite

La population est directement comptable **sur la surface elle-même** :

```sql
SELECT id, json_extract(metadata,'$.handler_failure.exit'), created_at
  FROM tasks WHERE json_extract(metadata,'$.handler_failure') IS NOT NULL
  ORDER BY created_at DESC;
```

Une seconde population dans `audit_events` serait à garder d'accord avec celle-ci pour toujours, pour un compte que cette requête rend déjà exact. L'AC4 demande une sonde sur la surface par task, pas un compteur.

### D7 — `_shared/` ne gagne pas de bibliothèque de handlers

Les quatre handlers reçoivent le même petit patron (D9) par édition, pas par extraction. Créer `_shared/handler-lib.sh` serait une abstraction dessinée sur quatre points dont trois ont déjà divergé, et `_shared/` est aujourd'hui le substrat de la boucle de dispatch — l'élargir est un risque sans commune mesure avec le gain. La moitié qui tient structurellement est de toute façon D1, côté moteur.

### D8 — `dispatch-lib.sh` n'est pas touché

Il porte déjà une machinerie supérieure (stderr scrubbé, trace tail mika#887, récupération du stdout, découverte de PR sur chemin crash). Son stderr de handler est couvert par D1 comme les autres. Le modifier est un risque disproportionné pour un gain nul.

### D9 — L'étape est une variable, posée aux sites, lue par le trap

```sh
_STEP="parse_input"        # posé avant le trap, donc jamais vide
_STEP_DETAIL=""
# …
_STEP="chdir"
cd "$CWD" || { _STEP_DETAIL="could not cd to $CWD"; echo "ERROR: could not cd to $CWD" >&2; exit 1; }
```

et dans `deliver_callback` :

```sh
if [ -z "$RESULT" ]; then
    RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}) at step '${_STEP}'."
    [ -n "$_STEP_DETAIL" ] && RESULT="${RESULT} ${_STEP_DETAIL}"
fi
```

Le détail couvre les échecs **prévus** ; le nom d'étape couvre les **imprévus**, qui sont ceux qu'on n'a par définition pas pensé à envelopper.

### D10 — R3 est réel pour un seul handler, et le plan le dit

Recensement de l'écart entre le parse de `TASK_ID` et `trap … EXIT` :

| handler | entre les deux | trou réel |
|---|---|---|
| `build-mika` | la définition de `deliver_callback` | marginal |
| `address-pr-comments` | idem | marginal |
| `resolve-pr-conflicts` | idem | marginal |
| **`deploy-mika`** | **la résolution de `CWD` (l.34-38)** | **oui** |

Seul `deploy-mika` exécute une étape réelle avant d'armer son trap. La correction reste appliquée aux quatre (poser `_STEP` dès l'entrée, déplacer toute résolution après le `trap`) parce qu'elle est bon marché et qu'elle rend la propriété vraie par construction plutôt que par chance. **Annoncer R3 comme fermant un trou béant sur les quatre serait faux ; c'est un trou sur un, et une garantie de forme sur les quatre.**

## Livrables

### R1 — Persister le stderr (`crates/mika-agent/`)

1. `task_engine/engine.rs` — constante `HANDLER_FAILURE_METADATA_KEY = "handler_failure"`, à côté de `PILOT_TRANSCRIPT_EXPECTED_KEY` et `DISPATCH_WORKTREE_FILE_KEY`.
2. `db/tasks.rs` — `set_task_handler_failure(&self, task_id, exit_display: &str, stderr: Option<&str>) -> Result<()>` : un `UPDATE … json_set(COALESCE(metadata,'{}'), '$.' || ?1, json(?2))`, payload sérialisé par `serde_json`.
3. `async_db.rs` — le miroir async.
4. `skills/executor.rs::spawn_long_running_exec` — dans `if !status.success()`, après la lecture du stderr et la construction de `code_display`, avant `update_task_failed` : scrub → tronque → écrit, fire-and-forget.
5. `crates/mika-cli/src/commands/tasks.rs::print_task_detail` — une ligne conditionnelle `Handler failure:` rendant `exit` et le stderr. Le JSON (`task_to_json`) porte déjà `metadata` et n'a rien à gagner.

### R2 — Nommer l'étape (`skills/bundled/`)

`build-mika`, `deploy-mika`, `address-pr-comments`, `resolve-pr-conflicts` : `_STEP` / `_STEP_DETAIL` (D9), étapes nommées par site (`parse_input`, `resolve_cwd`, `chdir`, `build`, `deliver`, et leurs équivalents par handler), message composé dans le trap avec le préfixe conservé (D5).

### R3 — Armer le trap tôt (`skills/bundled/`)

Les quatre : `_STEP="parse_input"` avant le parse ; `trap deliver_callback EXIT` immédiatement après le test `-z "$TASK_ID"` ; toute résolution (notamment le bloc `CWD` de `deploy-mika`) déplacée **après** le trap.

## Fire-Disposition

Ce plan livre **un** détecteur : `mika2532_the_handler_failure_key_has_a_single_writer` — scan de source refusant un second écrivain en production de `HANDLER_FAILURE_METADATA_KEY`.

**Disposition retenue : (a), exception nommée en allowlist — allowlist livrée VIDE.**

- **Zéro violation existante.** Le site est neuf ; il n'y a rien à exempter, donc aucune entrée à écrire. L'allowlist naît vide et un test frère, `mika2532_the_sole_writer_allowlist_is_empty`, la maintient vide : une allowlist née vide est sinon l'endroit où tombera la prochaine infraction (doctrine mika#2323).
- **Assertion auto-nettoyante.** Le scan échoue si la clé n'est écrite **nulle part** — un scan qui vise un nom mort se lit exactement comme un scan propre (mika#2103 / mika#2205). C'est le motif de `mika2496_the_cost_overrun_name_has_a_single_writer`.
- **Conduite quand il tire : on retire le second site, on ne l'allowliste pas** (doctrine mika#2201). La requête `SELECT … WHERE json_extract(metadata,'$.handler_failure') IS NOT NULL` de D6 **est** la mesure de la classe ; deux écrivains la rendraient inexacte sans qu'aucune assertion comportementale ne rougisse, ce qui est exactement pourquoi ce détecteur est un scan de source et pas un test.

**Un second détecteur a été envisagé et refusé** : un scan « tout handler long-running arme son trap avant toute étape ». Son prédicat serait lexical sur une propriété **positionnelle**, donc fragile dans les deux sens ; et sa valeur est faible puisque R1 couvre structurellement tout handler, présent et futur. Le refus est nommé ici plutôt que découvert plus tard.

## Contrat de vérification

### Rust — comportemental, `crates/mika-agent/tests/eval/test_handler_stderr_persisted_2532.rs`

`spawn_long_running_exec` est appelable directement (`pub(crate)`) avec un script `/bin/sh` temporaire : aucun réseau, aucun binaire externe, aucun serveur.

| test | forme | ce qu'il atteste |
|---|---|---|
| **T1 — le défaut mesuré** | row callback posée `completed`, script `echo "ERROR: could not cd to /nope" >&2; exit 1` | `handler_failure.stderr` contient `could not cd`, `.exit` vaut `Exit code: 1`, **et `tasks.result` est inchangé** |
| **T2 — contrôle négatif, succès** | `exit 0` | **aucun** `handler_failure`. Sans lui, « on écrit sur échec » est indistinguable de « on écrit toujours » |
| **T3 — non-régression cas A** | row `pending`, `echo boom >&2; exit 3` | `handler_failure` écrit **et** `tasks.result` porte l'erreur, statut `failed` |
| **T4 — contrôle négatif, secrets** | stderr portant une valeur en forme de jeton | la valeur est **absente** de la metadata |
| **T5 — stderr muet** | `exit 4` sans écrire | `.exit` présent, `stderr` **absent** (jamais `""`) |

### Rust — structurel

`mika2532_the_handler_failure_key_has_a_single_writer` + `…_the_sole_writer_allowlist_is_empty` (voir Fire-Disposition).

### Shell — `skills/bundled/_shared/tests/test_handler_crash_step.sh`

Un faux `mika` en tête de `PATH` capture l'argument du callback — motif de `test_stamp_pr_origin.sh`.

| test | forme | ce qu'il atteste |
|---|---|---|
| **S1 — rejeu exact** | `build-mika` avec un `cwd` inexistant | le RESULT capturé porte `HANDLER CRASH (exit code 1) at step 'chdir'` **et** le chemin fautif |
| **S2 — contrôle négatif** | chemin nominal | le RESULT capturé ne porte **pas** `HANDLER CRASH` |
| **S3 — forme, les quatre** | lecture de chaque handler | `trap … EXIT` précède tout site posant `_STEP=` autre que l'initialisation |
| **S4 — préfixe préservé** | S1 | le message commence toujours par `HANDLER CRASH` (D5, consommateur `self-dev-callback:162`) |

Enregistré au `Makefile` (cible `test-handler-crash-step`, voisine de `test-dispatch-lib`) et au job CI qui agrège les harnais shell.

**Chaque contrôle négatif doit être vu rouge** en neutralisant le terme qu'il vise, avant d'être déclaré vert.

## Surfaces opérateur

```bash
# 1. La cause d'un crash pré-résultat, sur la row elle-même
mika tasks get <task-id>            # ligne « Handler failure: »
mika tasks get <task-id> --format json | jq '.metadata | fromjson | .handler_failure'

# 2. Le handler a-t-il échoué, et sa cause a-t-elle été persistée ?
grep long_running_handler_exit_nonzero "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{task_id, code_display, task_was_terminal, stderr_bytes, stderr_persisted}'

# 3. CONTRÔLE — une persistance refusée (régime attendu : VIDE)
grep long_running_handler_failure_not_persisted "$MIKA_SPIRIT_LOG_FILE"
```

```sql
-- La population de la classe, sur la surface même (D6)
SELECT id, json_extract(metadata,'$.handler_failure.exit') AS exit_code, created_at
  FROM tasks WHERE json_extract(metadata,'$.handler_failure') IS NOT NULL
  ORDER BY created_at DESC;
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `long_running_handler_exit_nonzero` (`task_was_terminal: true`) | INFO | **non vide, faible** | le cas B. Chaque ligne est un crash dont la cause était jetée avant ce correctif |
| `long_running_handler_exit_nonzero` (`task_was_terminal: false`) | WARN | non vide, faible | le cas A, comportement inchangé |
| `long_running_handler_failure_not_persisted` | WARN | **vide** | toute occurrence est une `metadata` qui n'est pas du JSON valide (classe mika#2179) : la cause est de nouveau perdue |

Les niveaux existants ne bougent pas ; ce qui est ajouté est un **nom d'événement stable** sur les deux bras et les champs qui séparent les deux populations.

## Sondes post-déploiement, et leurs quatre haltes

**S1 — la surface existe (premier crash réel).** Un handler long-running qui sort non-zéro laisse `handler_failure` sur sa row. *Rejeu dirigé possible* : lancer `build_mika` avec un `cwd` inexistant.

> **Halte 1 — aucune ligne `long_running_handler_exit_nonzero` alors qu'un crash a eu lieu.** Ne pas élargir le prédicat par réflexe. `~/.mika/skills/` est une projection du **binaire**, pas du checkout : établir d'abord le déploiement (`cat ~/.mika/skills/.manifest-writer`, classe mika#2340), puis que le `mika-spirit` servi porte le correctif.

**S2 — l'étape est nommée (48 h).** Tout `HANDLER CRASH` rendu par les quatre handlers porte `at step '<…>'`.

> **Halte 2 — un `HANDLER CRASH` nu subsiste.** Un cinquième site porte le patron, ou `dispatch-lib.sh` a servi ce chemin (hors périmètre, D8). Établir **lequel** avant de toucher aux quatre.

**S3 — attribution, 30 jours.** La requête SQL de D6 donne la population de la classe. C'est elle, et non une intuition, qui conditionne l'ouverture d'un suivi sur la cause-racine restante.

> **Halte 3 — la population porte du trafic nominal** (plusieurs par jour, sur des handlers différents). Ce n'est pas l'observabilité qui est en cause : les handlers long-running crashent en série, et c'est **cela** qu'il faut traiter — mika#2536 pour la cause cwd, un ticket par cause ensuite.

**S4 — contrôle négatif de bruit (7 jours).** `handler_failure` est **absent** de toute row dont le handler a réussi.

> **Halte 4 — une occurrence sur un succès.** Le prédicat d'écriture a quitté la branche `!status.success()`. Désarmer par revert **avant** diagnostic : une cause d'échec inventée sur une row saine est un mensonge de même ordre que le silence qu'on répare.

## Ce que ce travail n'achète pas

- **Aucun crash n'est empêché.** La cause-racine du défaut mesuré (`MIKA_PLATFORM_DIR` scrubbé, donc `cd` sur un chemin inexistant) appartient à mika#2536. Ce travail rend la cause **lisible**, il ne la supprime pas — et la QA de PR #2530 continuera d'échouer jusqu'à ce que #2536 ferme la cause.
- **Aucune surveillance.** Les instruments sont les trois greps et la requête ci-dessus ; **leur silence ne prouve rien tant que personne ne les exécute**. La surface devient lisible ; elle ne devient pas surveillée.
- **Aucune rétro-persistance.** Les quatre crashes mesurés du 2026-09-25 n'auront jamais leur stderr : il n'existe plus. La sonde est la **prochaine** occurrence.
- **Aucune couverture d'un crash sans stderr.** Un handler tué par `SIGKILL` avant d'écrire quoi que ce soit laisse `exit` et rien d'autre — c'est honnête, et c'est la limite.

## Hors périmètre, délibérément

- **R4-R7** — cause-racine `MIKA_PLATFORM_DIR` et détecteurs cwd : enfant **mika#2536**, par décision opérateur du 2026-09-25.
- **`_shared/dispatch-lib.sh`** — D8.
- **Le scrub de `err_msg`.** L'exécuteur écrit aujourd'hui le stderr **non scrubbé** dans `tasks.result` sur le cas A, là où le shell scrub depuis mika#903. Trou réel, trouvé en chemin, **non corrigé ici** : sa population est différente (cas A), son blast radius est le `result` que le tour de callback consomme, et le corriger changerait un contenu que le modèle lit. **Suivi à ouvrir**, précondition : aucune — c'est un défaut de sécurité autonome. Le présent travail ne l'aggrave pas : sa propre surface est scrubbée (D4).
- **Une bibliothèque de handlers partagée** — D7, à rouvrir si un cinquième handler long-running apparaît.
- **Un détecteur de position du trap** — refusé avec sa raison en Fire-Disposition.

## Definition of Done

- [ ] `set_task_handler_failure` écrit `$.handler_failure` en un `UPDATE`, sur une row de n'importe quel statut.
- [ ] `spawn_long_running_exec` l'appelle sur `!status.success()`, sans condition de statut, scrub puis troncature à `MAX_OUTPUT_LEN`, fire-and-forget.
- [ ] `mika tasks get` rend une ligne `Handler failure:` quand la clé est présente.
- [ ] Les quatre handlers nomment leur étape et conservent le préfixe `HANDLER CRASH`.
- [ ] Les quatre arment leur trap avant toute étape ; le bloc `CWD` de `deploy-mika` est déplacé après.
- [ ] T1-T5 verts, chaque contrôle négatif **vu rouge** avant d'être vert.
- [ ] S1-S4 verts, enregistrés au `Makefile` et en CI.
- [ ] Le scan SOLE WRITER est vert, son allowlist est vide, son assertion anti-vacuité est en place.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test` propres.
- [ ] Cette entrée est reflétée dans `crates/mika-agent/CLAUDE.md` (§ Exec Handlers) — surfaces opérateur, régimes attendus, haltes.

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2532 (§ *Acceptance criteria (cœur R1-R3)*) :

1. **Persister le stderr d'un handler long-running en échec** vers une surface lisible **par task** (per-dispatch), y compris quand la tâche est déjà terminale — pour que la cause d'un crash pré-résultat soit diagnosticable. Sans ça, tout AC de correction est non vérifiable. (R1)
2. Le message de crash pré-résultat **nomme l'étape qui a échoué** et son détail (cwd / dépendance absente / parse), pas seulement « script failed before building result ». (R2)
3. Le **trap de livraison est installé dès que `TASK_ID` est connu**, pour qu'aucune étape postérieure n'échoue en silence. (R3)
4. Sonde : un crash pré-résultat d'un handler long-running laisse désormais son stderr sur une surface lisible par task ; test négatif → le stderr atteint bien `tasks.result` / la surface, en nommant l'étape.
