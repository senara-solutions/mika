# Exec Handlers

Moved verbatim out of `crates/mika-agent/CLAUDE.md` on 2026-10-13. That file is loaded whole into a pilot's prompt as soon as the pilot reads anything under `crates/mika-agent/`; at 594 KB it no longer fit, and `crates/mika-common/tests/claude_md_size.rs` now caps every `CLAUDE.md` in the repository. Headings keep their original level.


**Image protocol (`__mika_v1`):** Scripts return images via JSON envelope `{"__mika_v1": {"text": "...", "images": ["/path/to/img"]}}`. Executor validates files (5MB limit, magic-byte check for JPEG/PNG/GIF/WebP), base64-encodes, max 5 images per result.

**Long-running:** `long_running: true` + `estimated_duration_secs` in `skill.toml`. Conversation mode and `DeferredDispatch` silent mode (#1058). Creates callback task, injects `__mika_task_id` and `__mika_agent` env vars, spawns detached process. PID recorded for orphan cleanup. **Callback deferred dispatch (#1058):** When a callback or DeferredDispatch turn calls a long-running tool and `long_running_ctx` is `None`, the executor gate intercepts the call via `callback_task_id` on `ToolContext`. Instead of a hard error, it runs `check_lineage_cycle()` (lineage walk on `(repo, issue_number, skill)` tuple, max 4 hops, fail-open on extraction failure) and, if no cycle is detected, calls `register_deferred_callback()` to enqueue the dispatch. Returns `{"status": "deferred", "deferred": true}` so the LLM knows not to retry. The deferred callback fires as a `DeferredDispatch` silent turn which HAS `LongRunningContext` injected. Cycle detection rejects same-tuple re-dispatch (e.g., `groom-#159 → retry-groom-#159`) but allows cross-skill chains (e.g., `groom-#159 → pilot-#159`). **Dispatch-readiness guard (#525):** before spawning, `validate_dispatch_readiness()` enforces seven checks: (0) unauthorized webhook dispatch (#933) — if `originating_message` is present and matches the Webhook Fallthrough domain (`[GitHub]` prefix excluding ready-label, PR, and check-suite events), rejects with `unauthorized_webhook_dispatch` before any DB access. Pure string-prefix check, cheapest guard. Predicate shared via `crate::webhook_dispatch::is_unauthorized_webhook_dispatch()`. (1) task status must be `pending` or `in_progress` (rejects `blocked`/`completed`/`cancelled` with structured JSON error `task_not_dispatchable`), (2) no active callback child task may exist (rejects with `task_active_dispatch`), (3) no other task of the same dispatch class may have an active callback child — per-class slot guard (rejects with `global_dispatch_active`, scoped to `agent_id` + `dispatch_class`) (#583, #1001). `dispatch_class` is `'implement'` (dev-pilot, deploy_mika) or `'groom'` (dev-groom); pre-v34 NULL rows are treated as `'implement'` via SQL `COALESCE`. One implement + one groom dispatch may run concurrently per agent. The rejection JSON includes `blocker_kind` (`"real_callback"` or `"deferred_wrapper"`) and `blocking_label` for agent-native diagnostics (#1172 W3), (4) per-turn dispatch counter must be zero — only one long-running dispatch per agent turn (rejects with `dispatch_limit_exceeded`) (#583), (5) grooming-marker check (#919, #1108) — if the task's `reference_url` points to a GitHub issue AND the dispatch skill is `dev-pilot` AND `task.type == "issue"`, fetches the issue body via REST API and checks for three canonical grooming callouts: `> - **Branch:**`, `docs/plans/`, and a `second-pass` marker (canonical `(GROOMED)` or spec-tolerated `(READY, paraphrased GROOMED ...)`). Rejects with `dispatch_no_grooming_marker` (listing `missing_signals`) if any are absent. Bypass predicates: non-`dev-pilot` skill, non-issue task type, non-GitHub-issue reference_url, or `MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1` env var (WARN-logged). Fail-open when no `github_token` configured; fail-closed on API errors. Coupled pair with `skills/bundled/self-dev/system_prompt.md:253` (defense-in-depth prompt-level check), (6) GitHub `blockedBy` check — if the task's `reference_url` points to a GitHub issue, queries the GraphQL API for open blockers and rejects with `dispatch_blocked_by` if any are still open (#713). Fail-open when no `github_token` configured (check skipped with warning); fail-closed on API errors. Uses GraphQL variables (not string interpolation) for injection safety. `extract_open_blocker_numbers()` parses the response. `LongRunningContext` carries `dispatch_count: AtomicU32` initialized to 0 per turn; incremented after task creation and path validation, right before subprocess spawn. `LongRunningContext` also carries `originating_message: Option<String>` (#933) — populated from the latest user-role message in conversation mode, `None` for silent triggers. Fail-closed on DB errors. Auto-transitions `pending` tasks to `in_progress` on successful dispatch. Stricter than the shared `validate_task()` which also allows `blocked` for `delegate_task`. **Dispatch-rejection observability (#1108):** All 7 rejection sites write the structured JSON error to `tasks.result` via `record_dispatch_rejection()` (fire-and-forget, warn on DB failure). This surfaces rejection reasons to operator-visible surfaces (`mika tasks list`, dashboard task detail) without requiring DB-level inspection. The `write_task_dispatch_rejection()` DB method is agent-unscoped (keyed by `task_id` + `trigger_type = 'manual'`) because the earliest rejection site (unauthorized webhook) fires before the task is fetched.

### The stderr of a failed handler survives a terminal task (mika#2532)

**The defect was never a missing capture — it was a capture whose only
destination refused terminal rows.** `spawn_long_running_exec` has always read
the handler's stderr on `!status.success()` and built
`err_msg = "Process {code_display}: {stderr}"`, then handed it to
`update_task_failed`, whose `UPDATE` carries
`AND status NOT IN ('completed', …, 'delivered')`. A handler whose EXIT trap has
already delivered its callback leaves the row `completed`, so that write matched
nothing, the branch logged *"… but task already in terminal state"*, and the one
string naming the cause was dropped on the floor. Measured 2026-09-25:
`build-mika` crashed **4 times out of 4** during the QA of PR #2530, each attempt
rendering only `HANDLER CRASH (exit code 1). Script failed before building
result.` — so QA returned **COMMENTED** instead of approving and an implementation
stayed out of merge while its GitHub CI was green.

**Three cases, one broken.** Crash *before* the trap (no `TASK_ID`, missing `jq`)
leaves the row non-terminal → `Ok(true)`, the stderr **already** reached
`tasks.result`. Success → the branch is not taken. Crash *after* the trap and
before `RESULT` — the `cd` — leaves the row terminal → `Ok(false)`, and that is
the only case with no surface at all. The trap doing its job correctly is what
closed the door behind it.

**R1 — the remedy is engine-side, and that is what makes it a class fix.** Five
sites carry the `HANDLER CRASH` trap; the three that capture a `STDERR_FILE`
capture **claude-pilot's** stderr, never their own, and create that file ~200
lines *after* the trap. All five shared the hole; one site in Rust covers them
and every handler written later, `_shared/dispatch-lib.sh` included — which is
why that file is deliberately untouched.

> **Rectification (mika#2634): for `dispatch-lib.sh` the parenthesis this
> paragraph used to carry — *"the handler's `echo … >&2` goes to the inherited
> fd 2, i.e. the executor's pipe"* — is FALSE, and that is why the engine-side
> capture had nothing to persist on its pre-flight path.**
> `dispatch_claude_pilot` opens its trace with
> `exec 9>>"$TRACE_FILE" 2>/dev/null`, and an `exec` **without a command**
> applies its redirections to the current shell **permanently**: that
> `2>/dev/null` does not mask a failed fd-9 open, it replaces fd 2 for the whole
> rest of the handler. So that handler's stderr reaches `/dev/null`, never the
> executor's pipe, and `stderr_bytes: 0` on those rows is a property of the
> handler rather than a mute process.
>
> This was not a discovery either: `_halt_family`'s own comment states it
> textually (*"this function runs inside `dispatch_claude_pilot`, whose fd 2 is
> `/dev/null` from the moment `exec 9>>"$TRACE_FILE" 2>/dev/null` runs
> (mika#903)"*). What was missing was the link between that fact and *"the
> pre-flight writes its diagnostic into the void"* — the link mika#2634 had to
> make after three dispatches died over 2 h 45 reporting `stderr_bytes: 0`, read
> as "there was nothing to say".
>
> The channels that do work on that path are **fd 9** (the trace, which the EXIT
> trap appends to the callback on its crash arm) and **`RESULT`**, which the
> pre-flight now poses itself. The mika#2532 mechanism is untouched and correct;
> what changes is that it finally has something to persist here, because the
> pre-flight no longer exits before the trap exists. **Removing that permanent
> `2>/dev/null` is a named follow-up, not an oversight** — the executor reads
> that stderr only after `child.wait()`, so a handler writing more than a pipe's
> ~64 KiB on fd 2 would block indefinitely; lifting it means deciding where that
> stderr goes, and `TASK_ID` is not yet known at the line that opens fd 9.

The write is **unconditional**, not gated on the `Ok(false)` arm: *"what did this
process put on fd 2"* has nothing to do with the row's status. The surface is
`tasks.metadata` because [`Database::set_task_handler_failure`] carries no status
filter (`WHERE id = ?`) — *"`completed` is terminal, the metadata still writes"*
has been an explicit contract since #617, and is exactly the property AC1 asks
for. `tasks.result` is refused (it carries the message the callback turn consumes
and that `extract_callback_fields` / `parse_verdict` read), and a per-task
`.stderr` file too (the house has paid three times for a documented sink nothing
feeds — Signals M, Q, S). Payload in **one** `json_set`, `stderr` **omitted** when
fd 2 stayed mute — never `""` (mika#2331) — fire-and-forget, scrubbed then
truncated at `MAX_OUTPUT_LEN`. Full reasoning at each site's doc-comment.

**R2/R3 — the four handlers name their step.** `_STEP` / `_STEP_DETAIL`, set at
each site and read by the trap; the `HANDLER CRASH` prefix is **added to**, never
replaced (`self-dev-callback:162` documents it as a discriminant and
`dispatch-lib.sh` greps it), giving
`HANDLER CRASH (exit code 1) at step 'chdir': could not cd to /nope/mika`. The
detail covers the failures we saw coming; the step name covers the ones we did
not, which are by definition the ones nobody thought to wrap. The trap is armed
as soon as `TASK_ID` is known — **a real hole on one handler only**: `deploy-mika`
resolved its `CWD` before arming, and that block moved after. For the other three
the gap held only a function definition; the correction is applied to all four
because it is cheap and makes the property true by construction rather than by
luck.

**Operator surfaces.**

```bash
mika tasks get <task-id>            # `Handler failure:` + `Handler stderr:`
mika tasks get <task-id> --format json | jq '.metadata | fromjson | .handler_failure'

grep long_running_handler_exit_nonzero "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{task_id, code_display, task_was_terminal, stderr_bytes, stderr_persisted}'

# CONTROL — a refused persistence (expected regime: EMPTY)
grep long_running_handler_failure_not_persisted "$MIKA_SPIRIT_LOG_FILE"
```

```sql
-- The population of the class, on the surface itself
SELECT id, json_extract(metadata,'$.handler_failure.exit') AS exit_code, created_at
  FROM tasks WHERE json_extract(metadata,'$.handler_failure') IS NOT NULL
  ORDER BY created_at DESC;
```

| event | level | expected regime | reading |
|---|---|---|---|
| `long_running_handler_exit_nonzero` (`task_was_terminal: true`) | INFO | **non-empty, low** | the broken case. Each line is a crash whose cause was discarded before this fix |
| `long_running_handler_exit_nonzero` (`task_was_terminal: false`) | WARN | non-empty, low | the already-working case, behaviour unchanged |
| `long_running_handler_failure_not_persisted` | WARN | **empty** | any hit is a `metadata` that is not valid JSON (class mika#2179): the cause is lost again |

Levels do not move; what is added is a **stable event name on both arms** and the
fields that separate the two populations. **No `audit_events` row, deliberately:**
the population is directly countable on the surface itself by the query above, and
a second population would have to be kept in agreement with this one for ever, for
a count this query already makes exact.

**Post-deploy probes, and their four halts.**

- **S1 — the surface exists (first real crash).** A long-running handler exiting
  non-zero leaves `handler_failure` on its row. *Halt 1 — no
  `long_running_handler_exit_nonzero` line while a crash did happen:* do not
  widen the predicate by reflex. `~/.mika/skills/` is a projection of the
  **binary**, not of the checkout — establish the deployment first
  (`cat ~/.mika/skills/.manifest-writer`, class mika#2340), then that the served
  `mika-spirit` carries the fix.
- **S2 — the step is named (48 h).** Every `HANDLER CRASH` the four handlers
  render carries `at step '<…>'`. *Halt 2 — a bare `HANDLER CRASH` survives:* a
  fifth site carries the pattern, or `dispatch-lib.sh` served that path (out of
  scope). Establish **which** before touching the four.
- **S3 — attribution (30 days).** The SQL query above gives the population of the
  class; it, and not an intuition, is what conditions opening a follow-up on the
  remaining root cause. *Halt 3 — the population carries nominal traffic* (several
  a day, across different handlers): observability is not the subject — the
  long-running handlers are crashing in series, and **that** is what to treat
  (mika#2536 for the cwd cause, one ticket per cause after).
- **S4 — negative control (7 days).** `handler_failure` is **absent** from every
  row whose handler succeeded. *Halt 4 — one occurrence on a success:* the write
  predicate has left the `!status.success()` branch. Disarm by revert **before**
  diagnosis — a cause of failure invented on a healthy row is a lie of the same
  order as the silence being repaired.

**Guards.** `skills::executor::tests::mika2532::*` — five behavioural tests on a
real `/bin/sh` subprocess (the measured defect; **the success negative control**,
without which "we write on failure" is indistinguishable from "we always write";
non-regression of the live-row case; the scrub; the mute handler). They live
in-crate, not under `tests/eval/`: `spawn_long_running_exec` is `pub(crate)`, so
an integration crate cannot reach it, and widening a production function's
visibility for a test file's location would be the worse trade.
`canonical_tokens::tests::mika2532_the_handler_failure_key_has_a_single_writer` is
a source scan with an **empty allowlist** — the CLI renderer imports the constant
instead of retyping the literal, which is why it is `pub`. Shell side:
`skills/bundled/_shared/tests/test_handler_crash_step.sh` (`make
test-handler-crash-step`, CI-gated) replays the measured crash, keeps a negative
control on the nominal path, and asserts structurally that the trap precedes every
step a trap could cover — with its own two fixtures proving that check bites.

**What this does NOT buy.** No crash is prevented: the root cause of the measured
defect (`MIKA_PLATFORM_DIR` scrubbed, so `cd` lands on a path that does not exist)
belongs to **mika#2536**. Nothing is retro-persisted — the four measured crashes
will never have their stderr; the probe is the **next** occurrence. A handler
killed by `SIGKILL` before writing anything leaves `exit` and nothing else, which
is honest and is the limit. And no surveillance: the instruments are the greps and
the query above, whose **silence proves nothing until someone runs them**.

**Named out of scope, found on the way:** the executor writes the **un-scrubbed**
stderr into `tasks.result` on the non-terminal case, where the shell has scrubbed
since mika#903. A real hole, a different population, and its blast radius is the
`result` the callback turn consumes — **follow-up ticket**. This work does not
make it worse: its own surface is scrubbed.

### Un seul lecteur de la preuve de grooming (mika#2484)

`skills::executor::groomed_state(db, owner, repo, number, issue_body)` répond à
**« un `dev-pilot` peut-il partir sur ce ticket ? »** et il est le seul à le
faire. La porte (check 5 ci-dessus, via `evaluate_grooming_gate`) et le routage
du `ready_label_handler` en descendent tous deux.

**La divergence que ça ferme vivait à l'intérieur d'un même handler, à quatre
étapes d'écart.** L'étape 5 décidait `dev-pilot` vs `dev-groom` sur
`check_grooming_markers` **seul** — la forme du callout — sous un commentaire
qui affirmait « Same code path as `validate_dispatch_readiness` gate (#919) ».
C'était vrai en #919 ; depuis mika#1620 / mika#2287 la porte porte **deux**
couches, forme *et* preuve. Alors le handler choisissait `dev-pilot` puis
refusait à l'étape 9d le `dev-pilot` qu'il venait de choisir
(`dispatch_grooming_not_verified`), le ticket restait `ready`, et le tour
suivant recommençait. Mot pour mot la classe que mika#2158 a dû fermer un cran
plus haut. Depuis mika#2470 la Phase 2 d'`auto_pull` dispatche in-process par ce
même handler, donc **un seul site réparé couvre le webhook et le filet**.

**Quatre bras, et pas un booléen.** Trois causes mènent à `dev-groom` et
appellent trois lectures opérateur différentes :

| `GroomedState` | routage | ligne écrite |
|---|---|---|
| `Groomed` (callouts + preuve) | `dev-pilot` / `implement` | — (le chemin nominal reste muet) |
| `MarkersMissing` (premier grooming) | `dev-groom` / `groom` | inchangé — `note_degroomed_ticket` (mika#2242) |
| `MarkersWithoutProof` (groomé hors moteur, ou preuve purgée) | `dev-groom` / `groom` | `ready_label_markers_without_proof` |
| `ProofUnreadable` (base en panne) | `dev-groom` / `groom` | `ready_label_groom_proof_unreadable` |

Les fondre dans un `bool` rendrait la population mesurée de mika#2484
**incomptable**, et ferait lire une panne de base comme un succès du correctif —
même motif que `below_threshold` / `no_ready_label_event` (mika#2131) et
`in_flight_self_dev` / `live_pilot_orphaned_parent` (mika#2279). Les deux sites
de consommation font un `match` exhaustif **sans bras `_ =>`**.

**`check_grooming_markers` n'est pas touchée, et c'est structurel.**
`grooming_marker.rs` porte un test de parité entre `auto_pull::is_groomed` et
`check_grooming_markers(..).is_empty()` ; y intégrer la preuve le casserait — et
à raison : `is_groomed` répond de la **forme**, question à laquelle la base n'a
rien à dire et que le feeder pose légitimement sans elle. Deux questions, deux
noms ; la seconde appelle la première, jamais l'inverse (ce serait une
régression de mika#2120).

**Voisinage à ne pas confondre : le callout `Plan` a lui aussi un lecteur unique,
et il en a QUATRE appelants depuis mika#2608.** `crates/mika-agent/src/plan_callout.rs`
est la source de vérité du **chemin** porté par `> - **Plan:**`, et
`auto_pull::plan_ownership`, `dispatch-lib::_extract_plan_path` (phase 1, mika#2194),
`dispatch-lib::_committed_plan_on_branch` et le `elif` de
`dispatch-lib::_set_up_worktree` (phase 2, mika#2608) l'appellent tous sans en
garder de copie — l'inventaire des lecteurs bash est un **zéro** que
`test-dispatch-lib.sh` tient, et la seule occurrence du motif qui reste dans ce
fichier est son **écrivain**, `_write_canonical_callout`. C'est un axe distinct de
celui-ci : ici on demande *ce ticket a-t-il une preuve de grooming en base ?*, là
*quel chemin ce corps nomme-t-il ?* — et `check_grooming_markers`, qui lit la
**forme** par sous-chaîne, reste hors des deux unifications, par la même borne B1
dans les deux cas. Les deux sites bash que mika#2608 a basculés portaient une
tolérance décrite comme « strict, ancré » qui ne l'était pas : ils n'exigeaient
pas le littéral `docs/plans/`, ce qui faisait **tirer** la porte de grooming sur
un callout nommant `README.md` et bloquait le ticket en `already_groomed` de façon
permanente. Détail des quatre tolérances et de leurs deltas :
`docs/architecture/dispatch-lib-migration.md` § 3.

**Le feeder n'est pas touché non plus, et le DoD du ticket est écarté avec sa
raison.** Gater la promotion `ready` sur la preuve retirerait au ticket
callouté-sans-preuve **la seule route qui peut produire la preuve manquante** :
avec le routage corrigé, promouvoir `ready` déclenche un `dev-groom`. Le churn
mesuré n'était pas un excès de promotions, c'était une promotion qui aboutissait
au mauvais dispatch. On répare l'aboutissement.

**Et ce n'est pas une garantie de convergence — la limite est nommée.** Si le
plan résout encore sur la branche de dispatch, le `dev-groom` répond
`already_groomed` (mika#2012) et ne frappe **aucune** preuve : le ticket
repassera au tour suivant, borné par le budget de re-drive de mika#2020, au
même endroit qu'avant ce ticket. Ce que le routage achète est le cas où le plan
n'est plus sur la branche — là le groom converge. Coût assumé : cette tentative
consomme désormais un subprocess et le slot `groom`, là où l'étape 9d la
refusait à bon marché. Voir la halte 6 de la sonde S3 dans le `CLAUDE.md`
racine.

**`route_for` / `routing_note` sont des fonctions pures**, extraites du corps du
handler parce que le bras `ProofUnreadable` n'est pas atteignable de bout en
bout : une base en panne ferait aussi échouer la pré-création de la row **et**
l'écriture d'audit qu'on veut observer.

**Garde d'intention (R7/R8).** `validate_dispatch_readiness` refuse
`dispatch_grooming_intent_mismatch` quand `originating_message` commence par
`groom ` (insensible à la casse, blanc de tête toléré) **et** que le `skill`
demandé est `dev-pilot`. `webhook_dispatch::is_grooming_intent_message` est
ancré sur le **mot** : `starts_with("groom")` nu mordrait sur « grooming report
for mika#N », et le contrôle négatif est un test nommé. Pré-subprocess pour la
raison de mika#1646 — `run_claude_pilot` spawne un processus et crée un
worktree, donc une garde post-hoc *constate* le contournement de la porte de
preuve sans l'empêcher. Elle ne peut pas mordre sur les quatre chemins moteur,
et c'est **structurel** : `originating_message` vaut `None` sur l'auto-fire
post-groom (mika#1614) et sur tout tour de callback, et commence par `[GitHub]`
sur le webhook et la relance de verdict. Le refus porte sur le tour entier —
dériver le numéro d'issue ajouterait un second parseur là où le seul cas
légitime (la chaîne dev-groom → dev-pilot) ne passe pas par ce chemin.

**Gardes structurelles**, toutes avec **allowlist livrée vide** :
`canonical_tokens::tests::mika2484_un_seul_lecteur_decisionnel_de_la_preuve`
(scan à deux niveaux : un seul fichier hors plomberie lit la preuve, et dans ce
fichier une seule fonction), `…_les_noms_d_evenement_sont_un_format_de_fil`
(SOLE WRITER des deux `tool_name`), et
`ready_label_handler::tests::mika2484::…_le_routage_n_a_pas_de_bras_joker`.
Quand le premier tire, **on retire la lecture**, on n'ajoute pas d'entrée
(doctrine mika#2201). Aucun test comportemental ne peut voir ces classes : un
second lecteur ne rend aucune décision fausse *le jour où il est écrit*.

Surfaces opérateur, régimes attendus et haltes : `CLAUDE.md` racine
§ *Un callout de corps sans preuve en base route vers `groom`*.

### Un dispatch ouvert par un événement PR est borné à sa LIGNÉE (mika#2649)

`validate_dispatch_readiness` gagne un septième terme : dans un tour dont
l'`originating_message` est un `[GitHub] PR …` ou un `[GitHub] Check suite …`,
`run_claude_pilot` / `run_claude_pilot_groom` n'est autorisé que pour une tâche
**liée par lignée** à la cible de cet événement.

**Le trou, et il était structurel.** La garde (0) ci-dessus juge la **nature** de
l'événement source : les deux familles sortent du domaine Fallthrough parce que
`self-dev-webhook-qa` / `-ci` y portent des dispatchs légitimes. Une fois sorties,
**plus aucun terme ne liait le dispatch à la PR de l'événement.** Mesuré le
2026-10-02 (trace `17ba765a-be9c-11f1-94aa-c13d0500c506`) : un tour ouvert par une
revue QA sur la PR #2647 a lancé un implement de mika#2646 — un autre ticket, hors
fenêtre `ready`, pendant qu'un implement de #2636 volait. Deux pilotes implement
simultanés de 20:03:17Z à ~20:24Z.

#### Six rectifications que la lecture du code impose, et elles sont le premier livrable

**R1 — le cap implement EST vérifié au point de dispatch, sur les quatre chemins.**
`validate_dispatch_readiness` porte la garde par classe (#583, #1001, mika#2160) et
ses quatre appelants de production sont la frontière d'outil, le chemin ready-label,
le tick moteur et le verdict handler ; `try_acquire_dispatch_slot` est le **dernier**
terme de la même fonction et rend la revendication atomique (§ *The exec slot is
CLAIMED, not checked*). Le `run_claude_pilot` du constat **a traversé cette garde**.
Ce qui est faux n'est donc pas « le cap n'est pas vérifié » mais **ce que le cap
compte** : `has_active_callback_tasks_excluding` filtre
`t.status IN ('pending','in_progress')` — elle compte des **lignes**, et un pilote
est un **processus**. Un pilote vif dont la ligne callback est devenue terminale
(watchdog #959, supersession, `cancel_task`) est **invisible au cap**, ce qui est
exactement la topologie que `live_pilot` (mika#2279) existe pour lire et que la
garde du cap ne consulte pas. **Le mécanisme réel du contournement n'est pas
établissable depuis le bac à sable de dispatch** (`~/.mika/data/mika.db` n'y est pas
montée), donc l'AC2 du ticket sort de cette PR avec sa précondition écrite — la
sonde S5 du `CLAUDE.md` racine.

**R2 — AC1 seul referme l'occurrence mesurée, et c'est ce qui autorise le
découpage.** L'événement portait sur #2647, le dispatch visait #2646 : sous le terme
de lignée ce dispatch est refusé **avant** toute création de ligne callback et tout
spawn, donc les deux pilotes ne coexistent jamais, **quoi qu'ait fait le cap**.
*Un cap qu'on « répare » sans avoir établi par où il a fui est un cap qu'on élargit
au hasard sur un chemin qui marche.*

**R3 — les dispatchs CI-fix / QA-hold ne traversent pas la garde.** Ce sont des
dispatchs **moteur**, et le commentaire est au site même
(`server/verdict_handler.rs`) : *« `originating_message = None` because the dispatch
is engine-authorized … Passing `None` skips guard (0) »*. `iterate_dispatch`
(mika#2506) fait de même. Le contrôle positif que l'AC3 décrit est donc **hors de la
population par construction** : il passe parce qu'il n'est pas interrogé — d'où le
contrôle positif rectifié de R4.

**R4 — la population légitime du chemin LLM n'est PAS vide, et elle dispatche un
AUTRE ticket que la PR.** Le réflexe, une fois R3 lu, est de remettre
`[GitHub] PR ` dans le domaine Fallthrough. **Ce serait casser un flux prescrit** :
sur un `[GitHub] PR closed:`, la cascade de jalon M4 dispatche légitimement le
**frère pending suivant** (`skills/bundled/self-dev-webhook-qa/system_prompt.md`,
qui nomme même la garde en toutes lettres). **Donc le discriminant n'est pas
l'identité de la cible mais la LIGNÉE** — ce que l'occurrence mesurée ne partage pas
avec M4 :

| | M4 légitime | le constat |
|---|---|---|
| action de l'événement | `PR closed:` (mergée) | `PR review (…)` soumise |
| la tâche dispatchée | un **enfant pending préexistant** du parent jalon | une tâche **créée dans le tour même** |
| son `parent_task_id` | le parent jalon, partagé avec la PR | aucun |
| son `metadata` | porte la lignée du jalon | vide |

**R5 — deux lecteurs uniques existent, et il y en a un TROISIÈME que le plan
n'avait pas vu.** `deadline_verdict::parse_pr_target` est le lecteur unique de la
grammaire PR (mika#2368) et `worktree_reaper::issue_number_from_branch` celui du
numéro porté par une branche (mika#2619) — tous deux `pub`, tous deux **appelés**.
Le plan décrivait en plus « une extraction de `(branch: …)` » comme si aucun lecteur
n'existait : il en existe **deux** (`webhook_queue.rs` et `webhook_queue_v2.rs`, la
seconde se déclarant doublon assumé). `webhook_event_target` appelle donc
`webhook_queue_v2::classify_event` plutôt que d'en écrire une **troisième** copie —
la classe mika#2158 évitée à son troisième tour. Mesure qui décide de la forme du
prédicat check-suite : `parse_pr_target` rend **`None`** sur un texte check-suite
(épinglé à `deadline_verdict.rs`), parce que cette grammaire porte une **branche**
et pas un numéro de PR.

**R6 — `ToolContext` porte un booléen, jamais `originating_message`.** `tools/mod.rs`
(mika#2573) cite mika#2517 et tranche : ce qui traverse jusqu'aux outils est un
**verdict** (`is_webhook_fallthrough_turn: bool`), pas une `&str` avec sa charge
utile. Conséquence directe sur l'AC4 : `cancel_task`, `create_task` et
`update_task_status` sont des outils ordinaires dont le `ToolContext` **ne porte pas
la cible de l'événement**, donc les borner demanderait de filer une charge utile que
la maison a refusée deux fois. `validate_dispatch_readiness`, lui, **reçoit déjà**
`originating_message` **et** `tool_input` : AC1 est bon marché à ce site unique, et
nulle part ailleurs. AC4 est donc livré comme ce qu'il demande — un **recensement
avec un verdict par outil** (§ racine) — et non comme quatre gardes.

#### Le prédicat : une moitié pure, une traversée bornée

`webhook_dispatch::webhook_event_target(msg) -> WebhookEventTarget` est **pur**
(aucun I/O) et rend quatre états : `Pr`, `Branch { issue: Option<u64> }`,
`Unreadable` (préfixe présent, grammaire non parsée) et `NotApplicable` (hors
population). **Quatre et non trois**, ce qui rectifie le § 3.1 du plan : son § 4
énumérait quatre valeurs d'audit dont une pour le préfixe illisible, et les deux ne
pouvaient pas être vraies ensemble.

`skills::executor::evaluate_event_target_binding` fait la traversée, **quatre termes
évalués du moins cher au plus cher, le premier qui tient autorise** :

| # | terme | ce qu'il couvre |
|---|---|---|
| L1 | `task.reference_url` nomme la cible | la PR est elle-même la cible ; le cas `issue#N` d'un check-suite dont la branche porte `N` |
| L2 | `task.metadata.claude_pilot.pr_url` nomme la PR | la tâche **est** celle dont le dispatch a produit cette PR |
| L3 | un **frère** (même `parent_task_id` non nul) satisfait L1 ou L2 | **la cascade M4** |
| L4 | le **parent** satisfait L1 ou L2 | une re-tentative sous le parent jalon |

Le nominal (L1 ou L2 sur la tâche) ne coûte **aucune requête de plus** ; sans
`parent_task_id` — la forme exacte de la tâche du constat — le refus tombe sans
requête. L2 appelle `extract_pr_url`, qui existe déjà dans ce fichier et lit les deux
formes (imbriquée et à plat), donc rien n'est rendu `pub` et aucun troisième lecteur
n'est écrit.

**L1 est strict sur le TYPE de référence**, et c'est une décision : un événement PR
se compare à une `reference_url` de **pull request**, un check-suite à une
`reference_url` d'**issue**. Un numéro de PR et un numéro d'issue vivent dans le
même espace de numérotation GitHub mais désignent deux objets différents ; les
apparier serait une **coïncidence de numéro**, pas une lignée. Le cas réellement
fréquent — tâche sur `issues/2641`, événement sur la PR `pull/2647` — est couvert par
L2.

**Aucun appel réseau.** Résoudre `gh pr view --json closingIssuesReferences` pour
trancher « le ticket fermé par cette PR » est **refusé** : ce serait un aller-retour
GitHub de plus dans une fonction qui en fait déjà plusieurs à 10 s de timeout, sur un
chemin qui spawne un processus, et le lien est **déjà porté en base** par L2 —
`claude_pilot.pr_url` est estampillé par le producteur du dispatch. Doctrine maison :
*la cible est dite, jamais dérivée* (mika#2249, mika#2368).

#### Trois dispositions de fail-safe, et elles ne sont pas uniformes à dessein

1. **Cible d'événement illisible** ⇒ **autorise**. Il n'existe aucune cible à
   laquelle borner ; refuser serait refuser sur l'absence de question.
2. **Un terme de lignée illisible** (metadata absente, JSON non conforme,
   `reference_url` nulle) ⇒ ce terme **n'est pas satisfait**, les autres continuent
   d'être évalués. Doctrine `live_pilot` à la lettre — *un signal qu'on ne peut pas
   lire n'est jamais un terme satisfait* — et **c'est porteur ici** : la tâche du
   constat avait une metadata **vide**, donc « metadata absente ⇒ autorise » aurait
   autorisé l'occurrence mesurée.
3. **La question ne peut pas être posée du tout** (erreur base) ⇒ **refuse**, par
   cohérence avec la garde voisine `dispatch_check_failed` de la **même fonction**
   (*« Fail-closed: if we can't check global state, reject dispatch »*). Deux gardes
   d'une même fonction qui divergeraient sur l'erreur base seraient une dette de
   lecture.

**L'asymétrie de coût qui autorise le point 3, nommée :** un faux refus coûte un
dispatch — visible, nommé, écrit dans `tasks.result` par
`record_dispatch_rejection`, et re-drivé par le réconciliateur stuck-ready ; un faux
passage lance un pilote sur un ticket que personne n'a autorisé, brûle un créneau et
~20 min de travail. C'est l'**inverse** de l'arbitrage du faucheur mika#2420, et
l'arbitrage ne se transporte pas.

#### Placement, et il est le livrable autant que le prédicat

Dans `validate_dispatch_readiness`, **après** le fetch de la tâche (le prédicat en a
besoin) et **avant** les allers-retours GitHub de la porte de siège — donc avant tout
coût réseau — et **bien avant** la garde par classe, dont l'enqueue de callback
différé (mika#1011) ré-armerait et re-refuserait le refus à chaque replay, la raison
que ses deux voisines ont déjà dû écrire. Pré-hoc et non post-hoc, pour la raison de
mika#1646 : `run_claude_pilot` spawne un processus et crée un worktree, donc une
garde qui ne tire qu'après l'exécution **constate** la violation sans l'empêcher.

**Ne mord que sur les deux skills de pilote** (`PILOT_DISPATCH_SKILLS`, constante
nommée plutôt qu'un `matches!` au site d'appel) : `deploy_mika` atteint cette
fonction lui aussi et reste hors population — ce que la garde borne est le lancement
d'un **pilote**, pas tout travail long.

#### Surfaces, et un seul `tool_name`

Cinq noms d'événement de journal, chacun **au niveau de son régime attendu** — un
WARN sur une population nominale est un WARN qu'on finit par museler :
`webhook_dispatch_target_mismatch` (WARN, le refus), `webhook_dispatch_target_bound`
(INFO, le contrôle positif), `webhook_dispatch_lineage_unreadable` (WARN, le
fail-closed), `webhook_dispatch_event_unreadable` (WARN, grammaire bougée),
`webhook_dispatch_target_absent` (INFO, un check-suite sur `main` — **régime attendu
non vide**, et c'est pourquoi il a un nom à lui et pas celui de l'illisible ; motif
`below_threshold` / `no_ready_label_event`, mika#2131).

Un seul `tool_name` d'audit, `webhook_dispatch_target_binding`, la décision dans
`after_value` ∈ `{bound, refused, event_unreadable, no_target_in_event,
lineage_unreadable}` — motif `ready_label_outcome` (mika#2323), parce que les cinq
issues appartiennent au **même site** et à la **même population**, donc un
`GROUP BY after_value` les sépare et les rend soustractibles. **SOLE WRITER**,
épinglé par `canonical_tokens::tests::mika2649_le_nom_daudit_a_un_seul_ecrivain`,
allowlist livrée vide.

**Ce scan compare le littéral EXACTEMENT, et pas en sous-chaîne comme ses voisins —
fait mesuré en l'écrivant.** Le nom du résidu de cette famille est, par construction,
`<nom d'audit>_audit_failed`, donc le nom d'audit en est le **préfixe** : une
comparaison par sous-chaîne compterait ce résidu comme un second écrivain. Les scans
voisins (mika#2573 et ses semblables) emploient la sous-chaîne et sont verts **par
troncature accidentelle** de `production_sources`, pas par prédicat —
`builtin_handlers.rs:3105` porte bien `fallthrough_work_creation_audit_failed`, mais
ce fichier est tronqué à son `cfg(test)` de la ligne ~674. Leur réparation est un
changement de **leur** périmètre ; ce qui est à retenir est que la sous-chaîne n'est
pas le prédicat de référence de cette famille, c'est son accident.

#### Le recensement des lecteurs de grammaire, et la rectification de sa Fire-Disposition

`canonical_tokens::tests::mika2649_aucun_second_parseur_de_grammaire_devenement`
part de la **forme du parseur** (un `Regex::new(` dont un littéral porte `[GitHub]`)
et confronte sa population à `EVENT_GRAMMAR_PARSER_SITES` **dans les deux sens** —
un porteur hors recensement rougit, et une entrée dont le fichier ne porte plus de
regex rougit aussi, parce qu'un recensement qui survit à son site est un tiroir qui
se lit comme une couverture. Plus trois assertions auto-nettoyantes sur les trois
lecteurs que ce travail **appelle**.

**La Fire-Disposition du plan annonçait une allowlist livrée VIDE, et la mesure la
réfute :** cinq regex `^\[GitHub\]` sur trois fichiers, dont `CHECK_SUITE_RE` en
**double**. Une allowlist vide aurait rendu ce scan **rouge à la naissance** — et un
lint rouge à la naissance se fait désarmer. C'est donc un **recensement**, pas une
allowlist d'exemptions : la distinction que mika#2633 a déjà dû écrire — *on y
ajoute quand un nouveau lecteur est justifié, on n'y exempte pas un doublon*. Le
doublon existant y est **nommé**, précisément pour qu'un futur éditeur l'apprenne
avant d'en écrire un troisième.

**Effet collatéral mesuré :** les deux littéraux de préfixe ont désormais un site de
définition unique (`PR_EVENT_PREFIX`, `CHECK_SUITE_EVENT_PREFIX`), lu par les deux
faces de la même frontière — `is_webhook_fallthrough_domain`, qui sort la famille du
domaine, et `webhook_event_target`, qui l'y retrouve pour la borner. Ils sont
déclarés **en tête de module** et pas près de leur second lecteur, parce que
`production_sources` tronque au premier marqueur `cfg(test)` **commentaires compris**
et que `mika2517_the_fallthrough_domain_has_a_single_definition` les y confronte :
déclarés plus bas, ce scan devenait vert par troncature, et son anti-vacuité l'a dit
en rougissant.

#### Ce que ce travail n'achète PAS

- **Il ne répare pas le cap** (R1). Il rend le dispatch hors lignée impossible sur le
  chemin mesuré ; un contournement de cap par une autre cause reste ouvert, et la
  sonde S5 est ce qui le dimensionne — **suivi mika#2652**.
- **Il ne ferme pas `cancel_task` sur un pilote vif depuis un tour webhook** — le
  second cas de la même famille, mesuré le même jour (71 tours jetés sur `8a3b2082`).
  Son remède n'est pas un terme de cible mais un terme de **vivacité**, et son rayon
  de souffle est distinct : un faux refus d'annulation retire à l'opérateur son geste
  de reprise le plus court. **Suivi mika#2653.** Même famille, troisième cas :
  `promote_deferred_callback`, qui choisit quel dispatch prend le créneau —
  population **mesurée depuis** (2026-10-07 : 73 appels / 58 sessions, dont 27
  sessions de canal `github` pour 33 appels et 10 promotions réussies, toutes dans
  la lignée de l'événement), et **fermé par mika#2654** par un terme de classe de
  tour, la forme « borner à la lignée » ayant été écartée sur cette mesure même.
- **Il ne réduit PAS le résidu per-issue de mika#2155**, et il faut le dire parce que
  le voisinage invite à le croire : ce résidu est *deux dispatch-lib vivants sur le
  **même** ticket*, dont le premier sortant strippe le seat label du second. Un
  dispatch sur le même ticket **passe** L1/L2 par construction. Les deux termes sont
  sur des axes orthogonaux — `live_pilot_for_issue` répond *un pilote travaille-t-il
  sur ce ticket ?*, celui-ci répond *cet événement autorise-t-il un dispatch sur ce
  ticket ?* — et seul le premier adresse ce résidu.
- **Un tour webhook servi en mode silencieux échapperait à la garde** — borne héritée
  de mika#2517 / mika#2573, ni élargie ni modifiée, population mesurée vide (un tour
  silencieux a `originating_message = None`).
- **La détection de l'erreur base (disposition 3) n'est pas couverte par un test de
  bout en bout**, seulement sa **disposition** : `get_child_tasks` et `get_task`
  n'échouent pas sur une base en mémoire saine. Dit plutôt que découvert.

Surfaces opérateur, régimes attendus, recensement AC4 et les cinq haltes :
`CLAUDE.md` racine § *Un dispatch ouvert par un événement PR est borné à la lignée de
cet événement*.

### Une annulation depuis un tour webhook PR ne touche pas un pilote vif (mika#2653, phases A et B)

`CancelTaskTool::execute` gagne un terme : dans un tour dont
l'`originating_message` est un `[GitHub] PR …` ou un `[GitHub] Check suite …`,
l'annulation d'une tâche dont un pilote est **vif** est refusée — avant toute
écriture de statut et avant tout signal.

**Second cas mesuré de la famille mika#2649**, le même jour, et le plus coûteux :
trace `8ec6364c-be71-11f1-908b-e931f18d2c16`, 2026-10-02, un tour ouvert par une
revue QA sur la PR #2644 a appelé `cancel_task` sur le pilote Fix-CI en vol
(`8a3b2082`) — **71 tours jetés**. Ce qui sortait ce vecteur du périmètre de
mika#2649 est écrit dans le recensement AC4 de ce ticket : *son remède n'est pas
un terme de cible mais un terme de **vivacité***.

#### Six rectifications que la lecture du code impose, et elles sont le premier livrable

**R1 — le booléen existant est l'INVERSE de celui dont la garde a besoin.** Le
ticket dit *« juste un booléen de classe de tour sur le modèle
d'`is_webhook_fallthrough_turn` »*. Sur le **modèle**, oui ; par
**réutilisation**, non : `is_webhook_fallthrough_domain` **sort explicitement**
les deux familles qui nous intéressent du domaine Fallthrough — parce que
`self-dev-webhook-qa` / `-ci` y portent des dispatchs légitimes — donc le
quatrième booléen vaut `false` **exactement sur la population de ce ticket**, et
le lire serait une garde à population vide (classe mika#2205 : *une garde que
personne n'a exercée se lit exactement comme une garde qui marche*).
`is_webhook_pr_event_domain` est donc un **axe nouveau**, déclaré au même module
que les deux constantes de préfixe — leur site de définition est unique depuis
mika#2649 et un troisième lecteur *au même module* est exactement ce qu'elles
existent pour permettre. Les deux axes sont **mutuellement exclusifs par
construction**, épinglé par
`mika2653_les_deux_axes_de_tour_webhook_sont_exclusifs` sur deux corpus (celui de
la famille et celui de la frontière Fallthrough) : sans ce test, un futur éditeur
qui retirerait un `return false` du domaine Fallthrough créerait deux gardes qui
mordent sur la même population sans que personne ait tranché laquelle décide.

**Pourquoi un prédicat de préfixe et pas `webhook_event_target`.** La sémantique
du second serait juste — `Unreadable` (préfixe présent, grammaire non parsée) est
**dans** la population, un tour webhook PR dont la grammaire a bougé restant un
tour webhook PR — mais il paierait deux regex par tour de conversation pour n'en
lire qu'un booléen, et jetterait la cible. L'accord des deux faces est **épinglé**
à la place (`mika2653_le_predicat_de_prefixe_saccorde_avec_la_cible`), motif
`mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`.

**R2 — `live_pilot_for_issue` n'est pas appelable ici.** Il prend une **URL
d'issue** ; `cancel_task` reçoit un **identifiant de tâche**, et la topologie à
deux lignes que `live_pilot` documente lui-même interdit de dériver l'une de
l'autre : une ligne callback — celle qui porte le pgid, donc le cas probable du
constat — **n'a pas d'URL**. `live_pilot_for_task` est le lecteur manquant, et il
vit **dans `live_pilot.rs`** : son contrat écrit (*« **Sole reader** of that
question »*) passe de deux appelants à trois plutôt que d'être recopié. *Un
recensement n'est pas une allowlist : on y ajoute, on n'y exempte pas*
(mika#2633).

**R3 — la traversée que ce lecteur demande EXISTAIT DÉJÀ**, dans
`cancel_task_and_kill` (posée par mika#2335). **AC6, livré en phase B** :
`live_pilot::resolve_task_pilot` est le site unique de la traversée
parent→enfant d'un id de tâche, appelé par `cancel_task_and_kill` **et** par
`live_pilot_for_task`. Il rend des **candidats** (`TaskPilot::Named`,
`Children`, `ChildrenUnreadable`) ; la **classification reste à chaque appelant**,
et elle diverge délibérément : le chemin de kill **écarte** un enfant sans
`process_start_time` (il ne signale pas un groupe de processus qu'il ne peut pas
identifier), le verdict le rend `Unreadable` (il ne peut pas prouver l'absence de
pilote). La traversée n'écarte donc **rien** qu'un des deux lecteurs garde. Les
deux filtres de la phase A (terminal sur la ligne nommée, start_time lu par
`Database::process_start_time_from_metadata`) s'appliquent désormais aussi au
kill — sans changement d'issue sur le terminal (`cancel_task` refuse une ligne
terminale), et dans le sens **sûr** sur la forme entière du start_time (le kill
vérifie l'instance au lieu d'une existence nue : population signalée plus
étroite, jamais plus large).

Le scan `canonical_tokens::mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique`
refuse tout fichier de production hors `live_pilot.rs` qui **appelle** les trois
termes de la question : une traversée de dispatch, `is_terminal_task_status` et
`is_same_process_alive`. Allowlist `LIVE_PILOT_TASK_READER_ALLOWED` livrée vide
et épinglée vide ; anti-vacuité sur `live_pilot.rs` ; contrôle de bonne foi sur la
forme réelle du `process_kill.rs` de la phase A (vu rouge sur cet arbre).
**Prémisse du plan corrigée :** la composition traversée × preuve d'instance
n'était pas absente de l'arbre — `TaskEngine::dispatch_liveness` (`engine.rs`,
mika#2156) la porte. Elle pose une autre question (*un processus tourne-t-il
encore sous cette ligne de suivi, quel que soit le statut de l'enfant ?*) et
refuse délibérément le filtre terminal (D-2 : 1146/1147 enfants porteurs de pid
sont `delivered`). C'est pourquoi le terme terminal est dans le prédicat ; le
résidu est nommé au test : un futur lecteur sans filtre terminal échapperait au
scan, mais ne répondrait plus à la question de `live_pilot.rs`. Ce résidu n'est
pas seulement futur : `probe_pilot_liveness` (`mika-cli`, `mika tasks show`) lit
déjà la même vivacité sans filtre terminal (il affiche, il ne décide rien) ; le
router par `live_pilot` demande d'exposer `resolve_task_pilot` hors du crate, et
c'est un suivi.

**R4 — un site couvre deux outils.** `CancelReminderTool::execute` est
littéralement `CancelTaskTool.execute(input, ctx).await`, donc la garde couvre
`cancel_reminder` **gratuitement**. À nommer plutôt qu'à découvrir : un futur
éditeur qui dédoublerait `cancel_reminder` perdrait la garde en silence.

**R4-bis — les deux poignées doivent appliquer les MÊMES filtres, et la
première rédaction ne le faisait pas.** Trouvé en revue, et les deux écarts
étaient réels. *(a)* Le filtre terminal manquait sur la poignée « ligne
nommée » : une ligne callback `delivered` portant encore un pgid vif rendait
`Alive`, donc la garde refusait l'annulation sous `blocked_live_pilot` — une
population documentée comme attendue-zéro — sur une ligne que
`cancel_task_and_kill` aurait de toute façon déclinée par son honnête *« not in
cancellable status »*. La fenêtre est réelle : le trap EXIT de `dispatch-lib`
livre le callback depuis l'**intérieur** du wrapper encore vivant, donc
`delivered` précède la sortie du processus. *(b)* La lecture de
`process_start_time` passait par un `serde_json` inline acceptant la seule forme
chaîne, là où la poignée « enfant » passe par le lecteur de colonne qui accepte
**aussi** la forme entière — donc une seule fonction répondait deux choses de la
même ligne, et sous un fail-closed cet écart refuse d'un côté ce qu'il laisse
passer de l'autre. La règle a désormais un site partagé
(`Database::process_start_time_from_metadata`). Les deux corrections sont
**épinglées et vues rouges par mutation**
(`mika2653_une_ligne_terminale_nest_pas_un_pilote` et
`mika2653_les_deux_poignees_lisent_le_start_time_par_la_meme_regle`, chacun
interrogeant les **deux** poignées — l'asymétrie du test reproduisait exactement
celle du code).

**R5 — le geste de reprise de l'opérateur n'est PAS sur ce chemin**, et c'est la
rectification qui décide l'arbitrage. Les quatre appelants de
`cancel_task_and_kill` et leur traversée du `ToolContext`, plus la raison pour
laquelle un `mika ask` d'opérateur est hors population : `CLAUDE.md` racine
§ *`cancel_task` n'annule pas un pilote vif depuis un tour webhook PR*.

**R6 — deux vecteurs voisins ne se ferment pas par ce terme** :
`update_task_status → cancelled` (qui **orpheline** le pilote au lieu de le tuer
— dommage différent, remède différent) et `run_shell` (population mesurée vide).
Tous deux **suivis nommés**, et c'est pourquoi le corps du refus ne nomme **aucune
commande**.

#### Conception

`webhook_dispatch::is_webhook_pr_event_domain` (pur) →
`ToolContext.is_webhook_pr_event_turn` (cinquième booléen, calculé au **seul** site
de conversation par **appel** du prédicat, `false` aux trois autres sites de
production) → `live_pilot::live_pilot_for_task` (le verdict, dans son module
propriétaire) → `tools::cancel_task::classify_cancel_on_live_pilot` (le prédicat
pur) → la garde.

**Le terme de classe de tour est un PARAMÈTRE du prédicat pur**, jamais une
branche de l'appelant — forme mika#2649 : « hors tour webhook PR, rien ne change »
devient ainsi une propriété de la fonction, avec son propre test
(`mika2653_hors_tour_webhook_pr_rien_ne_change`), plutôt qu'un `if` à lire.

**Quatre issues, `match` exhaustif sans bras `_ =>`** : `Allowed` (hors
population — aucune ligne, aucun audit, doctrine mika#2131),
`AllowedInPopulation` (le contrôle positif), `RefusedLivePilot`,
`RefusedUnreadable`.

**Placement :** après `AgentScopedTaskId::from_tool_context` et
`validate_task_exists` (la tâche doit exister et appartenir à l'agent avant qu'on
parle de son pilote), **avant** `cancel_task_and_kill` — donc avant toute écriture
de statut et avant tout signal. **Dans l'outil, jamais dans
`cancel_task_and_kill`** : c'est ce qui préserve les trois chemins opérateur par
construction plutôt que par prédicat. Le verdict de vivacité n'est résolu que
**dans la population**, donc le trafic hors population ne paie aucune requête.

**L'arbitrage de fail-safe est fail-CLOSED**, l'inverse de la politique propre de
`live_pilot` pour ses deux appelants existants (mika#2279, qui a délibérément
dérogé à mika#2277). Ce n'est pas une doctrine qu'on applique, c'est une
**asymétrie de coût** qu'on refait site par site — la table, et le coût nommé du
choix, sont dans la section racine.

#### Quatre détecteurs

- **1 — `mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique`** (phase B, AC6 ;
  `canonical_tokens.rs`, allowlist `LIVE_PILOT_TASK_READER_ALLOWED` livrée vide et
  épinglée vide). Voir le paragraphe R3 ci-dessus pour son prédicat à trois
  termes et les deux lecteurs sans filtre terminal qu'il ne voit pas.
- **2 — `mika2653_le_nom_daudit_a_un_seul_ecrivain`** (`canonical_tokens.rs`,
  allowlist livrée **vide** et épinglée vide). La comparaison du littéral est
  **exacte et non en sous-chaîne**, reprise mot pour mot de la correction mesurée
  par mika#2649 puis mika#2634 sur leurs propres scans jumeaux : le nom du résidu
  (`cancel_task_pilot_guard_audit_failed`) a le nom d'audit pour **préfixe**, donc
  une sous-chaîne le compterait comme un second écrivain. Ces deux surfaces sont
  distinctes — l'une répond à « combien d'annulations la garde a-t-elle
  tranchées », l'autre à « la ligne d'audit est-elle passée » — et aucune n'a à se
  taire pour que l'autre soit exacte. Son **contrôle de bonne foi**
  (`mika2653_le_scan_du_nom_daudit_voit_un_second_site`) est dû : resserrer une
  garde sans montrer qu'elle mord encore est la façon dont une garde devient
  inerte en silence. **Aucun test comportemental ne peut voir cette classe** : un
  second écrivain ne rendrait aucune décision fausse, seul le **compte**
  deviendrait inexact.
- **3 — format de fil**, deux volets :
  `mika2653_les_valeurs_daudit_sont_un_format_de_fil` (les quatre `after_value`,
  **dans les deux directions** — tout ce qui est produit est déclaré, et tout ce
  qui est déclaré est atteignable ; sans la seconde, une valeur pourrait être
  groupée par un opérateur et produite par rien) et
  `mika2653_les_motifs_derreur_sont_un_format_de_fil` (les valeurs du champ
  `error`, qui atterrissent dans `tool_calls.output`).
- **4 — exclusivité des deux axes** (`webhook_dispatch.rs`), c'est AC5.
- **5 — la garde d'exécution** : les unités de `classify_cancel_on_live_pilot`
  avec leur **contrôle négatif hors population** (porteur : sans lui, « la garde
  décide » est indistinguable de « la garde bloque tout »), plus
  `tests/eval/test_cancel_task_live_pilot_2653.rs` sur le chemin de production.

#### Ce que les tests attestent, et ce qu'ils ne peuvent pas

Les unités de `classify_cancel_on_live_pilot` restent **vertes** si le champ
`is_webhook_pr_event_turn` n'est jamais calculé ou jamais lu — et c'est très
exactement le mode de panne, puisque le booléen est posé à **un** site de
production et que les dix-neuf autres le mettent à `false`. Seul le fichier eval
ferme ça. **Vérifié par mutation à la livraison** : en remplaçant l'appel du
prédicat par `false` au site de conversation, V6a / V6c / V6d rougissent et V6b
(le contrôle négatif) reste vert — ce qui est la forme attendue et ce qui dit que
le câblage n'est pas vide.

Le pilote des tests eval est un **processus enfant**, jamais celui du binaire de
test, et la raison est porteuse : le contrôle négatif V6b **traverse**
`cancel_task_and_kill` par conception, lequel envoie un SIGTERM au **groupe de
processus** du pgid enregistré. Avec `std::process::id()` — la forme que
`live_pilot.rs` emploie sans danger pour ses propres unités, aucune n'atteignant le
chemin de kill — ce contrôle tuerait la campagne qui l'exécute, de façon
dépendante de l'environnement.

Surfaces opérateur, régimes attendus, recensement AC4, les cinq haltes et ce que
ce travail n'achète pas : `CLAUDE.md` racine § *`cancel_task` n'annule pas un
pilote vif depuis un tour webhook PR*.

**Cancel discriminator protocol (#749):** When `cancel_task_and_kill` terminates a long-running subprocess, it pre-writes a reason file at `/tmp/mika-cancel-reason-{pid}` with `STATUS=CANCELLED_BY_OPERATOR` before sending SIGTERM. The shell-side TERM trap in `dispatch-lib.sh` writes `STATUS=CANCELLED_BY_SIGNAL` only if no reason file exists (belt-and-suspenders for signal-initiated cancels). The EXIT trap reads the reason file and prefixes the callback envelope so the consumer (`self-dev-callback`) can distinguish cancel from crash. Two discriminators: `CANCELLED_BY_OPERATOR` (cancel_task initiated) and `CANCELLED_BY_SIGNAL` (signal-initiated, no pre-write). Absence of the prefix = existing `HANDLER CRASH` / success paths fire unchanged (backward compatible).

### Un tour webhook PR ne force pas la promotion d'un wrapper différé (mika#2654)

`PromoteDeferredCallbackTool::execute` gagne un terme : dans un tour dont
l'`originating_message` est un `[GitHub] PR …` ou un `[GitHub] Check suite …`,
forcer la promotion du prochain wrapper différé est **refusé** — avant toute
lecture et toute écriture en base.

**Troisième et dernier cas nommé par le recensement AC4 de mika#2649**, et le seul
des trois qu'aucun incident n'a exercé : il est là parce que la lecture du code
montrait qu'il est servi dans ces tours et qu'il **choisit quel dispatch prend le
créneau** de sa classe. Les trois axes de la famille sont distincts — mika#2649 un
terme de **lignée**, mika#2653 un terme de **vivacité**, celui-ci un terme de
**classe de tour**, sans terme de cible.

#### La mesure, et c'est elle qui a décidé la forme

Le ticket s'est ouvert sur une précondition explicite : *établir que la population
existe avant d'armer quoi que ce soit*, parce qu'un prédicat sur une population
vide produit un détecteur dont le silence ne prouve rien (classe mika#2205). Deux
mesures d'opérateur en lecture seule sur `~/.mika/data/mika.db`, le 2026-10-07,
fenêtre 2026-09-07 → 2026-10-07, l'ont levée : **73 appels** à
`promote_deferred_callback` sur **58 sessions**, dont **27 sessions (31 appels)**
de canal `github` ; sur ces tours, 33 appels et **10 réussis** (16 « No pending
deferred wrapper », 7 `slot_busy`).

La seconde moitié de cette mesure est ce qui a **écarté** la forme que le ticket
proposait en premier. Les 10 promotions réussies sont **toutes** le fait de
mika-qa et promeuvent **toutes** un wrapper dont le parent est la vérification de
build **de la PR de l'événement** — #2250 (cinq promotions en 31 s sur le même
parent), #2273, #2393, #2406, #2416, #2661. **Zéro promotion hors lignée
mesurée.** Borner à la lignée comme mika#2649 aurait donc autorisé 10 sur 10 : sa
population de refus serait **vide**, et ce détecteur inerte aurait été payé au
prix de la charge utile que mika#2517 puis mika#2573 ont refusée deux fois.

**Limite de la mesure, dite plutôt que découverte.** Le texte d'origine de ces
tours n'est pas persisté dans `messages` pour le canal `github` : la classe de
l'événement s'y lit au **premier outil** du tour (`qa_pr_view` / `pr list` sur une
PR précise, pour au moins 7 des 27), pas au message. La garde teste le **préfixe
du message**, le compte testait le **canal** — `channel_type = 'github'` est donc
un **sur-ensemble** de la population que la garde reconnaît. Cet écart est ce que
la sonde S2 mesure après déploiement, et il est la raison de la halte 1.

#### Six rectifications que la lecture du code impose au corps du ticket

**R1 — l'outil n'est pas un échappatoire « quand le créneau est pris »**, et le
corps du ticket décrivait la fonction inverse de celle que le code remplit.
`Database::force_promote_deferred_for_class` (`db/tasks.rs`) **refuse** quand la
classe est au plafond (`ForcePromoteResult::RejectedSlotBusy`). Quand elle ne
l'est pas, il fait ce que le backstop périodique `promote_pending_deferred_if_idle`
(`task_engine/engine.rs`) fait de lui-même au scan suivant :
`DB_SCAN_INTERVAL_TICKS = 60`, soit **≤ 60 s**.

**R2 — ce que l'outil ajoute au backstop, et c'est le vrai dommage.** Le backstop
consulte la priorité opérateur (`has_pending_operator_task_for_class`, mika#1948
AC3) et **cède la place** quand l'opérateur a du travail `pending` dans la classe.
`force_promote_deferred_for_class` **ne la consulte pas**. Depuis un tour webhook,
l'outil n'achète donc que deux choses : une avance ≤ 60 s et le contournement de
cette priorité — plus le churn mesuré sur #2250.

**R3 — la raison qui faisait préférer la retenue à mika#2517 ne tient plus.**
mika#2517 retenait `create_task` parce que refuser à la frontière d'outil demandait
un champ que `ToolContext` n'avait pas. Depuis mika#2653,
`ToolContext.is_webhook_pr_event_turn` existe, calculé au **seul** site de
conversation par **appel** de `webhook_dispatch::is_webhook_pr_event_domain`. La
forme retenue ne coûte donc **aucune** plomberie : le booléen est déjà là, déjà lu
par une garde sœur, et cette PR en ajoute un **lecteur**, pas un champ.

**R4 — l'outil ne prend pas de cible.** Son unique paramètre est `dispatch_class`
(`enum: ["implement","groom"]`) et il promeut « le prochain wrapper » de la classe.
Borner à la lignée voudrait dire résoudre *quel* wrapper serait promu, le
confronter à la cible de l'événement par la traversée de mika#2649, et changer
l'API de la promotion. Ce n'est pas un terme ajouté : c'est un **second sélecteur
de wrapper**.

**R5 — aucun prompt ne prescrit l'outil**, et c'est la condition qui rend le refus
non bouclant. Dans le dépôt il n'apparaît que dans le code, la documentation et
des plans : aucun `system_prompt.md`, aucun `skill.toml`, aucun handler ne le cite,
et aucune garde `required_tools` ne le réclame. Les appels mesurés sont
**spontanés**, émis après un `{"status":"deferred"}` rendu par l'exécuteur.

**R6 — refus et non retenue, alors que la retenue serait ici tenable.** mika#2653
a écarté `FALLTHROUGH_WITHHELD_TOOLS` parce que le prompt prescrit l'annulation
d'un dispatch réellement perdu ; ici rien ne prescrit rien (R5), donc l'argument ne
s'applique pas. Ce qui tranche est que **le refus EST la mesure** : chaque
tentative devient une ligne `tool_calls.output` et une ligne d'audit, là où retenir
l'outil du tableau ferait disparaître la population et obligerait à inventer une
ligne par tour webhook PR pour la garder observable. Ce ticket est né d'une
exigence de mesure ; il ne pouvait pas se refermer sur une forme muette.

#### Conception

`webhook_dispatch::is_webhook_pr_event_domain` (pur, inchangé) →
`ToolContext.is_webhook_pr_event_turn` (le champ de mika#2653, **inchangé**) →
`tools::promote_deferred_callback::classify_force_promote_turn` (le prédicat pur)
→ la garde dans `execute`.

**Le terme de classe de tour est un PARAMÈTRE du prédicat pur**, jamais une branche
de l'appelant — forme mika#2649 reprise par mika#2653 : « hors tour webhook PR,
rien ne change » devient une propriété **testée de la fonction**
(`mika2654_hors_tour_webhook_pr_la_promotion_passe`,
`mika2654_la_table_de_verite_du_classifieur`) plutôt qu'un `if` à relire. La
conséquence est assumée et nommée dans le doc-comment : l'appelant branche de toute
façon sur le drapeau, donc en production la branche `Allowed` du prédicat n'est pas
atteignable.

**Deux issues et pas trois, contrairement à la sœur.**
`ForcePromoteTurnDecision { Allowed, RefusedWebhookPrTurn }` n'a **pas** de
`AllowedInPopulation`. `CancelPilotDecision` en a un parce qu'elle porte un second
terme — la vivacité d'un pilote — donc « la garde a tourné et a laissé passer » y
est un état distinct qui doit être compté. Ici le seul terme **est** la classe de
tour : dans la population, la garde refuse toujours, et un `AllowedInPopulation`
serait une valeur d'audit que rien n'écrit — une colonne de zéros qui se lit comme
une population saine. Le contrôle positif de cette garde vit donc **hors du type**,
dans l'eval AC9 (un tour `[GitHub] Issue …` qui passe).

**Placement :** **après** la validation de `dispatch_class` — une entrée malformée
lit la même erreur qu'ailleurs, le refus ne la masque pas — et **avant**
`force_promote_deferred_for_class`, donc avant toute lecture et toute écriture en
base. C'est l'ordre qu'AC2 exige : un tour PR dont la classe est au plafond est
refusé sous le motif **webhook**, jamais sous `slot_busy`, parce que le second
lirait comme un problème de créneau et orienterait vers le forçage. **Dans l'outil,
jamais dans `force_promote_deferred_for_class`** : c'est ce qui préserve par
construction le CLI `mika tasks promote-deferred` et le backstop périodique, qui
partagent cette fonction et ne traversent aucun `ToolContext`.

`report_force_promote_turn_decision` est appelée **inconditionnellement** et rend
`()` : c'est la fonction elle-même qui sort sans rien écrire hors de la population
(doctrine mika#2131, et c'est ce qui rend la non-régression d'AC3 *observable* —
zéro ligne d'audit sur un tour d'opérateur), et sa signature est ce qui garantit
qu'un échec d'audit ne peut pas remonter jusqu'au verdict (motif
`report_cancel_pilot_decision`). Un `if` au site d'appel répéterait le prédicat et
ouvrirait un second endroit où la borne peut diverger de celle du refus.

**Le corps du refus nomme la levée sans donner de gabarit** (doctrine mika#2520 /
mika#2292) : le fait (ce tour a été ouvert par un événement PR / check-suite), ce
que forcer achèterait réellement (une avance d'un scan, le contournement de la
priorité opérateur), le fait que **le moteur promeut le wrapper de lui-même** au
prochain scan, et la sortie correcte (terminer le tour sur ce que l'événement
demande). Il **ne nomme aucune commande** — ni le geste CLI de promotion, ni son
drapeau de forçage, ni `run_shell`, ni `update_task_status` : `MIKA_DEV_IDENTITY`
porte `shell-exec`, donc écrire la commande ici donnerait le gabarit du
contournement. Liste interdite épinglée par
`mika2654_le_refus_ne_nomme_aucun_contournement`.

#### Ce que les tests attestent, et ce qu'ils ne peuvent pas

Les unités posent `ctx.is_webhook_pr_event_turn` **à la main**, donc elles restent
**vertes** si le champ n'est jamais calculé ou jamais lu — et c'est exactement le
mode de panne de cette famille, puisque le booléen est posé à **un** site de
production et `false` à tous les autres. Seul
`tests/eval/test_promote_deferred_webhook_pr_2654.rs` ferme ça. **Vérifié par
mutation à la livraison** : en posant `is_webhook_pr_event_turn: false` au site de
conversation, les deux cas de refus de l'eval rougissent (chacun rendant *« Deferred
wrapper promoted »*) et les dix unités restent vertes — la forme attendue, et ce qui
dit que le câblage n'est pas vide.

Les **deux** cas de refus sont porteurs parce que `is_webhook_pr_event_domain` est
une **disjonction** : un correctif qui ne câblerait qu'un des deux préfixes
laisserait l'autre ouvert avec le premier cas vert. Et le cas `[GitHub] Issue …`
épingle que le domaine Webhook Fallthrough reste hors périmètre **par décision** —
`FALLTHROUGH_WITHHELD_TOOLS` ne retient que `create_task`, donc l'outil y est
réellement servi et le test le montre en train de promouvoir.

Le scan SOLE WRITER du nom d'audit
(`canonical_tokens::tests::mika2654_le_nom_daudit_a_un_seul_ecrivain`, allowlist
`FORCE_PROMOTE_TURN_GUARD_SOLE_WRITER_EXCEPTIONS` livrée **vide** et épinglée vide)
compare le littéral **entier** et **exactement**, jamais en sous-chaîne : les trois
noms frères de mika#1453 partagent le préfixe `deferred_dispatch_force_promote`,
donc un scan sur ce préfixe serait rouge à la naissance — et un lint rouge à la
naissance se fait désarmer. Son contrôle de bonne foi
(`mika2654_le_scan_du_nom_daudit_voit_un_second_site`) est dû pour la raison que
mika#2649, mika#2634 puis mika#2653 ont dû écrire chacun sur son scan jumeau :
resserrer une garde sans montrer qu'elle mord encore est la façon dont une garde
devient inerte en silence. **Aucun test comportemental ne peut voir cette classe** :
un second écrivain ne rendrait aucune décision fausse, seul le **compte** de la
sonde S1 deviendrait inexact.

#### Ce que ce travail n'achète PAS

- **Il ne retire rien à l'opérateur.** `mika tasks promote-deferred <class>` ne
  traverse aucun `ToolContext`, donc aucun prédicat de ce niveau ne peut
  l'atteindre ; et un `mika ask` d'opérateur reste hors population, son message
  étant son propre texte, qui ne commence pas par le préfixe.
- **Il ne ferme pas les tours Webhook Fallthrough**, et c'est une décision :
  population mesurée **1** tentative, échouée, en 30 j. Armer une garde sur une
  population quasi vide ne prouverait rien (mika#2205). Suivi à ouvrir seulement
  si une mesure montre une tentative **réussie** ; AC9 épingle la frontière par un
  cas qui **passe**.
- **Il ne corrige pas l'absence de la priorité opérateur dans
  `force_promote_deferred_for_class`** (R2). Elle touche aussi le CLI, où ce
  contournement est un geste d'opérateur **légitime** : décision distincte, et elle
  appartient au CLI autant qu'au moteur.
- **Il ne touche pas le backstop.** `promote_pending_deferred_if_idle` est inchangé,
  octet pour octet — c'est lui qui promeut le wrapper refusé, et sous **deux**
  contrôles au lieu de zéro.
- **Un tour webhook servi en mode silencieux échapperait à la garde** — borne
  héritée de mika#2517 / mika#2573, ni élargie ni modifiée, population mesurée vide
  (un tour silencieux a `originating_message = None`).
- **Il ne rend pas l'exclusivité des deux axes structurelle.** Le suivi nommé par
  mika#2653 — réunir les deux booléens en un `webhook_turn: WebhookTurnClass`
  (`None | Fallthrough | PrEvent`) — gagne ici un second lecteur à migrer, et reste
  un suivi.

Surfaces opérateur, régimes attendus, sondes S1–S3 et leurs haltes :
[`docs/operator/environment-variables.md`](../operator/environment-variables.md)
§ *`promote_deferred_callback` est refusé sur un tour webhook PR*.

### Platform-root relay, and the name it translates (mika#2536)

`inject_platform_dir_env` is the **sixth** injector in `spawn_long_running_exec`,
beside `inject_pilot_transcript_env`, `inject_dispatch_worktree_env`,
`inject_rescue_verify_env`, `inject_arch_ask_retry_env` and
`inject_pilot_dispatch_env`. Same placement contract as all five — it MUST run
**after** `sandboxed_pilot_env`, whose `env_clear()` would erase it — and the same
best-effort discipline: absence or an empty value is a no-op, never a blocked
dispatch.

**It is the only one that TRANSLATES the name, and that is the whole point.** It
reads `MIKA_PLATFORM_DIR` spirit-side and poses `PLATFORM_DIR`
(`PLATFORM_DIR_RELAY_KEY`) child-side. `inject_pilot_dispatch_env` relays
`std::env::var(k)` under the *same* `k`, so copying its shape would have required
the operator to rename a variable mika#2491 documents — and *a setting that stops
being read without an error* is the exact defect mika#2536 closes, moved one notch.
The motif followed is `inject_pilot_transcript_env`
(`MIKA_LOG_PILOT_TRANSCRIPTS` → `ANTHROPIC_LOG_FILE`).

**What was inert, and on both exec paths.** The ten sites across five scripts read
`${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}`. `sandboxed_pilot_env`
rebuilds the child env from a positive allowlist that refuses every `MIKA_*`
(plus a `debug_assert` against adding one), and the non-long-running exec path
runs `scrub_mika_env_vars`, a denylist on the same prefix. So the branch could
only ever take its fallback — **a dead branch on every path**, which is why
`${MIKA_PLATFORM_DIR:-…}` is a *class*, not one site.

**`DISPATCH_ENV_KNOWN_INERT` decreased for the first time**, which is the effect
its own doc-comment promises (*"when an entry is settled, we RELAY it and remove
its line"*). The removal was not optional: the test's self-cleaning assertion
(`population.contains(name)`) **requires** it the moment `dispatch-lib.sh` stops
reading the prefixed name. Its sibling term (`!reaches_dispatch_child(name)`) is
why `reaches_dispatch_child` had to learn `PLATFORM_DIR_RELAY_KEY` — the shell
reads it self-referentially (`PLATFORM_DIR="${PLATFORM_DIR:-…}"`), so term 4bis of
the predicate puts it **in** the population, where it would otherwise read as an
orphan while traversing perfectly.

**Two class scans, partitioning one population.**
`mika2536_no_handler_reads_a_variable_that_cannot_reach_it` covers
`skills/bundled/*/handlers/*.sh` **concatenated with the `_shared/` libraries they
source**; `mika2508_every_operator_var_read_by_dispatch_lib_reaches_the_child_or_is_named`
covers `dispatch-lib.sh`. Eight of the ten corrected sites are in the first, two in
the second. Three properties are worth knowing before touching either:

- **The concatenation is a correctness condition, found by measurement.**
  `external_env_reads` tracks writes *within one file*, so a variable written by a
  sourced library and read by the handler looks external. First run of the new
  scan: six false positives (`CWD_REFUSAL`, four `PR_PUSH_GUARD_*`). The remedy is
  to scan the script **as it executes**, never an allowlist entry.
- **`dispatch-lib.sh` is excluded as a PERIMETER, not an allowlist.** It has its own
  scan and its own *named* inertia list; concatenating it would surface those three
  entries in the new scan, whose allowlist is empty by contract — the scan would be
  born red, and a lint red at birth gets disarmed. The exclusion is compared both
  ways (`mika2536_the_t1_perimeter_names_only_files_that_exist`), because a
  perimeter that outlives its file is a drawer.
- **`HANDLER_ENV_KNOWN_INERT` is shipped empty and pinned empty.** When the scan
  fires, route the site to the relay; do not exempt it. If an inertia genuinely must
  be named, its place is `DISPATCH_ENV_KNOWN_INERT`, with its reason and its
  follow-up.

Neither scan is replaceable by a behavioural test: a dead branch makes **no
decision wrong**, it makes a setting inoperative in silence, and every existing
assertion stays green. Same reason `mika2536_the_relay_runs_after_the_env_sandbox`
is a source scan — an injector moved above `sandboxed_pilot_env` breaks nothing
either, it just goes back to the default.

Config, the four named `cwd` refusals, the operator query and the four halts: root
`CLAUDE.md` § *`MIKA_PLATFORM_DIR` traverse enfin*.

### The refusal is universal BY SITE, never by enumeration (mika#2639 AC4)

`run_shell` refuses to install into the host's Python environment — the fourth
lexical scan of `shell-exec/handlers/run.sh`, after mika#1957 and mika#1991,
before the shared-checkout guard (mika#2449). Measured producer: mika-qa ran
`pip install -e . --break-system-packages` against a claude-pilot worktree on
2026-10-02; its `[console_scripts]` entry point rewrote
`~/.local/bin/claude-pilot` and killed every pilot at launch for 2 h 45
(mika#2634).

**AC4 is answered by the SITE.** Which agents carry `shell-exec`:

| allowlist | site | agent |
|---|---|---|
| `DEFAULT_AGENT_SKILL_ALLOWLIST` | `mika-common/src/home.rs` | `mika` personal / customer (tier `default`) |
| `DEFAULT_IDENTITY` (TOML mirror; `home.rs` tests pin the equality) | `mika-common/src/home.rs` | idem |
| `MIKA_DEV_IDENTITY` | `well_known_agents.rs` | mika-dev |
| `MIKA_QA_IDENTITY` | `well_known_agents.rs` | mika-qa |

Absent from `FAMILY_AGENT_SKILL_ALLOWLIST` (tiers `family`, `champion`),
`MIKA_TEST_IDENTITY` and `MIKA_ARCH_SKILL_ALLOWLIST`. The plan's fifth row,
`MIKA_RELAY_IDENTITY`, exists nowhere in `crates/` and is dropped; § *Skills
System* still names it (pre-existing staleness, out of scope).

**The real population is wider than those rows**, and that decided the site: an
`identity.toml` with no `[skills].allowlist` is a no-op in
`apply_identity_allowlist`, so every bundled skill — `shell-exec` included — is
active (mika#1596 class). A per-agent refusal would be false by construction;
the table is documentation, never a mechanism.

**Two motifs, one definition site** (`_refuse_python_installer`, stderr +
`exit 1`): `host_installer` (`pip`/`pipx`/`python -m pip` + install
subcommand, or `uv tool install|upgrade`, after venv-qualified forms **in
command position** are neutralised — a venv path passed as an option value,
`python3 -X .venv/bin/python -m pip`, is not the interpreter invoked) and
`host_target_flag` (`--break-system-packages`/`--user`/`--target`/`--prefix`/
`--system` or its `PIP_*=` environment twin, **and** an install/develop verb
**and** a named installer — `pip`/`pipx`/`uv`/`setup.py`; two terms refused
`./configure --prefix=/usr && make install` — plus the persistent
`pip config set <section>.break-system-packages|user|target|prefix`). The
gap between binary and subcommand admits one separate option argument
(`pip --cache-dir X install`); the review of the first version found each of
these escapes, and `scripts/test-python-installer-guard.sh` pins every one.

**`tmux` stays uncovered.** `tmux/handlers/create_session.sh` sends a
model-supplied `$COMMAND` through `send-keys`, outside `run.sh`, and `tmux` is
in mika-dev's and mika-qa's allowlists. Population unmeasured; follow-up gated
on root probe S4.

The substitute (`uv run`, or a venv under the worktree) is in
`qa-review/system_prompt.md` next to **Never compile inside the review turn**;
the prompt expresses the intent, this block holds it. Harness
`scripts/test-python-installer-guard.sh`; operator surfaces and probes: root
`CLAUDE.md` § *`shell-exec` refuse d'installer dans l'environnement Python de
l'hôte*.
