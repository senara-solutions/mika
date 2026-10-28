# Milestone Manager (Phase 1)

Moved verbatim out of `crates/mika-agent/CLAUDE.md` on 2026-10-13. That file is loaded whole into a pilot's prompt as soon as the pilot reads anything under `crates/mika-agent/`; at 594 KB it no longer fit, and `crates/mika-common/tests/claude_md_size.rs` now caps every `CLAUDE.md` in the repository. Headings keep their original level.


`src/milestone_manager/` — milestone-scope operational coordinator (`mika-manager` entity, distinct from `mika-prime`). Ratified 2026-08-21 by Vincent + Prime (5 verdicts, brief at `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`). **LECTURE seule** — zero dispatch, zero ticket mutation, zero PR merge. **Two** outbound side effects, both towards the control-monitor and neither a forge write: a report `POST` to a well-known delivery endpoint (Prime→sami→Vincent per D8 subsystem-2 pattern) or an offline sink write when the URL is unset, and — since mika#1990 — a **liveness beat**, one light `POST` per successful poll tick (§ *Liveness beat* below). Both are declared in `docs/egress/egress-manifest.toml`.

**Three composers + one loop.** `Reader` (`reader.rs`) wraps `gh` CLI (`api`/`issue list`/`pr list`) mirroring the `auto_pull::gh_list_open_issues` subprocess shape and composes `MilestoneState` (sub-issues + progress counts + recent activity). `Assessor` (`assessor.rs`) applies four rules (stale-blocker, silent-progress, silence-in-JOURS, priority ranking) and classifies `Severity` (Healthy/Attention/Blocked). `Reporter` (`reporter.rs`) formats the § 2d Markdown report. `run_manager_cycle` in `cadence.rs` orchestrates read→assess→deliver with hybrid cadence: event-driven trigger on `state_digest` change + 6h plancher heartbeat (« l'absence d'event EST l'event »).

**Wrapper-only INV-2.** No new GitHub API client dep; `Reader` uses `tokio::process::Command::new("gh")` verbatim from `auto_pull.rs`. Delivery uses the existing workspace `reqwest` — bearer-token POST to `MIKA_MANAGER_DELIVERY_URL` (normal) or `MIKA_MANAGER_ESCALATION_URL` (Severity::Blocked → Vincent-direct route). Both URLs unset → offline sink at `MIKA_MANAGER_OFFLINE_SINK_DIR` so nothing is lost during bring-up. **That variable name is the one the code reads, and this line used to carry a different one** (`MIKA_MANAGER_SINK_DIR`, which exists nowhere) while the line six paragraphs down spelled it correctly — the class mika#1971 named in as many words: *a deployer who followed it put the key in the one place it could not work*. The occurrence is fixed; the class is closed by `mika2267_every_manager_env_const_is_declared_in_env_example`, which refuses an `ENV_*` const the code reads and `.env.example` does not declare.

**Structural LECTURE-seule enforcement.** `no_dispatch_test.rs` greps the module tree for forbidden write-authority tokens (`run_claude_pilot`, `pr_merge_with_gate`, `gh api "PATCH"`, `gh issue edit`, …) and fails the test binary if any executable code contains them. Prompt-level "no dispatch" rules are fragile per `feedback_prompt_enforcement_fragile` — this test is the structural gate. Comments are stripped before scanning so doc prose can describe what's forbidden.

**CLI surface.** `mika milestone {read,assess,report} <owner/repo>#<number>` — thin adapter in `crates/mika-cli/src/commands/milestone.rs`. Report subcommand emits Markdown to stdout; `read`/`assess` default to pretty JSON (also YAML via `--format yaml`).

### The offline sink has a reader (mika#2267)

**The defect, and the rectification the code imposed on the ticket.** mika#2267 read the symptom as a broken cm delivery channel (*"sink offline vs endpoint, à déterminer"*) and offered two branches: repair the delivery, or document the fallback sink. Reading the code moves the diagnosis one notch, and that displacement is the first deliverable. **The channel was not a pierced pipe: the fallback sink had no reader at all.** Exhaustive search over the tree returned `offline_sink` / `OFFLINE_SINK` at exactly three sites — the write (`cadence.rs`), the config (`spawn.rs`), the prose (this file). Zero CLI command, zero tool, zero HTTP route, zero runbook, zero consumer. From the point of view of the human who owes the Reader fidelity verdict, **"the channel is broken" and "the channel is a well" produce exactly the same bytes: none.** Same class as mika#2205 (*a silently inert scan reads exactly like a scan that found nothing to do*) and mika#2131's mute exclusions.

**Scope constraint, stated rather than discovered.** `control-monitor` is **not in this workspace** (it holds `claude-pilot/` and `mika/` only), so no line of this repo can make a cm endpoint exist or make it live. This work takes the second branch the ticket explicitly authorises — and makes it better than documentation by shipping a **reader**. Prose saying "reports are in `~/.mika/manager/sink/`" leaves the verdict hanging on an `ls` someone has to remember; a subcommand makes it executable and, above all, **names the path it consulted**, which is the only way the CLI-and-daemon-disagree case becomes readable in one line instead of an investigation. Repairing the cm delivery stays a **follow-up ticket**, and its precondition is the measurement this work produces: until one knows whether a URL was ever posted, "repair the channel" is repairing blind.

**`mika milestone reports`** — `--target <owner/repo>#<n>` restricts to one milestone, `--latest` prints the most recent report's Markdown, `--limit` (default 20) caps the listing, `--format text|json|yaml`. **The load-bearing property is that an absent directory is not an empty one:** `SinkListing` has three variants (`DirAbsent` / `Empty` / `Entries`), never a bare empty list, because the two say opposite things — *the cadence has never written HERE* against *the cadence wrote here and the sink was emptied* — and collapsing them would reproduce, at the reader's own surface, the silence it exists to lift. The output **always names the consulted path and its provenance**, on the nominal path too.

**One resolver, and the guard that keeps it one.** The path used to be composed inline in `manager_config_from_env`. A CLI reader recomposing it would be free to diverge from the writer — **and a reader that looks somewhere the writer does not is precisely the defect being closed, reproduced one layer up**. `milestone_manager::sink_dir::resolve_offline_sink_dir() -> (PathBuf, SinkDirSource)` is the sole site; writer and reader both go through it. `mika2267_sink_dir_resolution_has_a_single_reader` is a source scan over `crates/mika-agent/src` **and** `crates/mika-cli/src` refusing two needles — the env-var **literal** (a second reader relying on its own copy of the name) and the `.join("sink")` recomposition (a second reader refabricating the default path). **Allowlist shipped empty:** when it fires, remove the second site, do not allowlist it. No behavioural test can see that class — a second resolver makes no decision wrong the day it is written; it diverges later, in silence, with every assertion still green.

**`manager_delivery_resolved` (INFO, once per cadence start)** — the line that settles the ticket's own question without reading the source. Fields: `milestone`, `route_normal` / `route_escalation` (`"http"` | `"offline_sink"` — what a non-`Blocked` and a `Blocked` severity will respectively take), `delivery_url_set`, `escalation_url_set`, `delivery_token_present`, `offline_sink_dir` (the **resolved** path), `sink_dir_source` (`"env"` | `"default"`). A distinct event rather than more fields on `manager_cadence_start`: it is a *configuration* event answering "where do my reports go?", not "the cadence started", and it must grep alone — exact precedent and identical reasoning, `llm_budget_resolved` (mika#2293), *un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir*. `delivery_token_present` is a **boolean** — never the value, never a prefix, never a length (doctrine `api_key_present`, asserted by a negative test that sweeps *every* field, because a leak arrives through the field nobody thought to look at). **Its absence while the cadence runs means the deployed binary predates the fix (class mika#2340)** — establish the deployment before concluding anything about the channel; never read the silence as "all is well".

**`route` is a wire format, and the fallback stopped being mute.** Three constants, one site (`ROUTE_HTTP` / `ROUTE_OFFLINE_SINK` / `ROUTE_OFFLINE_SINK_FALLBACK`), pinned by test. The post-HTTP-failure fallback was **the only one of the three write paths emitting nothing**: it wrote the sink, set `delivered = true`, and left no line saying a report had been written or where. It now emits `manager_cycle_delivered` under `offline_sink_fallback`, **deliberately distinct** from `offline_sink`: the two populations say opposite things — *no URL was posted* (nominal bring-up, nothing is broken) against *a URL was posted and failed* (a real fault) — and merging them would erase the exact distinction the ticket asks to draw. Two crossed tests hold it: asserting only that the failure emits *a* line would be satisfied by reusing `offline_sink`.

**What this does NOT change.** `CycleOutcome::delivered` keeps its meaning: `true` on all three paths. It is an output field read by tests and telemetry, and hardening it would change a public contract for a gain `manager_delivery_resolved` and the fallback route already deliver. What was missing was never a stricter boolean — **it was knowing where.** A future ticket that genuinely wants to tell "read" from "written" must first define what "read" means; that is not a boolean question. The sink **format** is unchanged too (`write_offline_sink` still writes `report_markdown` alone, not the full `DeliveryBody`), with the named consequence that **the reader cannot filter by severity** — it lives in the report body, not in a machine header. Writing the full JSON would be richer but would change the format of files already on disk in production, and nothing in the ticket asks for that filter; date ordering covers the measured need, which is reading the latest report. A named limit, not an oversight.

**Operator probes, and their halts.** (1) `grep manager_delivery_resolved "$MIKA_SPIRIT_LOG_FILE" | jq '{route_normal, route_escalation, delivery_url_set, offline_sink_dir, sink_dir_source}'` — `route_normal: "offline_sink"` means **no URL was ever posted**: the reports have been on disk from the start, the cm channel never existed, and the remedy is a configuration gesture plus the cm follow-up ticket, **not a code fix**. `route_normal: "http"` sends you to probe 2. **Halt — no line at all while the cadence runs:** class mika#2340, establish the deployment first. (2) `grep manager_cycle_delivered "$MIKA_SPIRIT_LOG_FILE" | jq -r .route | sort | uniq -c` — `http` dominant means delivery works and the symptom is **downstream** (the cm consumer does not render the reports): **halt, do not touch this repo**. `offline_sink_fallback` dominant means the endpoint refuses, and `manager_cycle_delivery_failed` carries the error; the cm follow-up then opens **with a count rather than an intuition**. (3) `mika milestone reports --latest` must return a non-empty Phase 1 report — **that is the criterion unblocking the Reader fidelity verdict**. **Halt — empty output while probe 1 says `offline_sink`:** the CLI and the daemon do not resolve the same path (different `HOME`, service under another user); compare the printed path against probe 1's `offline_sink_dir`, which is exactly why the reader names it. **Expected regimes:** `manager_delivery_resolved` one line per start, always present; `offline_sink_fallback` **zero in a healthy regime** — any occurrence is a configured endpoint refusing.

**Out of scope, named.** The sink has **no rotation** and grows unbounded; the reader makes the volume visible, which is the precondition for deciding on rotation — a follow-up ticket if the probe shows a problematic volume. And `emit_auth_alarm` (`spawn.rs`) still writes **nothing** to the sink when `escalation_url` is unset: it returns early and only the `error!` survives. A real hole, found on the way and deliberately **distinct** — a lost report is not a lost alarm, and fixing it requires deciding what an alarm in a well even means. **Follow-up ticket.**

### Liveness beat — the freshness beats at the poll's pace (mika#1990)

`milestone_manager/liveness.rs` — one light `POST` per **successful** poll tick on
`MIKA_MANAGER_LIVENESS_URL`, so the cm registry sees a 5-min sign of life instead of
one every 6 hours.

**The defect.** The manager polls every 5 min and posted **nothing** until it
delivered — delivery being hybrid (`state_changed || heartbeat_fired`, the second on
a **6 h** floor). Between two legitimate beats the registry received no sign of life,
the entity's freshness went RED, and the nudge-scanner would cry wolf on its first
scan while the cadence was running perfectly.

**A distinct channel, never a field on `DeliveryBody`.** That struct is a wire format
cm consumes, and the *freshness* does not read in a 30 KB report posted four times a
day. Distinct endpoint, minimal body, one beat per tick — and `reason` says *why this
beat happened*, which is what makes AC3 readable literally: N poll ticks + 1 delivery
= N+1 beats, exactly one carrying `delivery:<severity>`.

**The URL is DECLARED, never derived** (mika#2249 / mika#2368 doctrine). Not composed
from `delivery_url` (`…/api/v1/messages/dispatch` — unrelated shape), nor from
`health_url` (`…/agents/mika-dev/health`), whose entity is not ours and whose meaning
is **inverse**: there we *read* the executor's health, here we *write* our own.

**Why not `…HEARTBEAT_URL`.** `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS` already exists and
names the **6 h delivery floor**. Side by side in the same `EnvironmentFile`, a
`…HEARTBEAT_URL` would read as *"the heartbeat to that URL beats every 6 hours"* —
precisely the false belief this ticket exists to kill. The endpoint path keeps the word
(the ticket names it); the variable does not. Pinned by
`mika1990_le_nom_de_la_variable_ne_dit_pas_heartbeat`.

**On the `Ok` arm, and only there.** A cycle that fails — typically `gh` in 401 — is a
**broken** manager; posting *"I am alive"* there would be the exact lie freshness must
not tell. That half already has its channel (`manager_cycle_error`, and the mika#2013
`manager_auth_persistent_failure` alarm). The beat says *"the cadence runs **and** reads
GitHub"*, which is stronger than *"the process exists"*.

**`<n>` counts loop iterations, not beats emitted.** Incremented at the head of the
iteration, **by the call site** rather than by `beat` — otherwise a failing cycle, which
does not beat, would not increment and the holes would vanish. A run of `poll:5` →
`poll:9` therefore says *"four cycles failed"*. The counter restarts at `1` on process
start, so `poll:1` marks a restart — useful, not a defect.

**`reason` is a wire format.** The values land in the cm registry and an operator will
`GROUP BY` them, so two spellings of one motive would split a population without saying
so. Two constants, **one composition site** (`liveness_reason`), `severity` rendered by
the same `snake_case` as `serde` — pinned against serde's real output by
`mika1990_la_severite_du_motif_suit_serde`, because the `match` alone could drift from
the `rename_all` attribute.

**The failure class comes from the STATUS, never from the message text** (mika#2179
rule): `CredentialRefused` (401 | 403), `Unreachable` (`is_connect() || is_timeout()`),
`Other`.

**Best-effort, and the signature is what guarantees it (AC1).** `beat` returns nothing,
so no caller has any way to let a lost beat break its loop. Bounded by `LIVENESS_TIMEOUT`
(5 s, aligned on `probe_executor_health` — same nature, a liveness probe runs short),
i.e. at worst **1.7 %** of a 300 s poll. Emitted **after** the cycle, so it delays no
cycle of its own; it can delay the *next* one, and `tokio::time::interval` catches up a
late tick. A detached `tokio::spawn` was declined: it would make AC1 trivially true at
the cost of the beats' ordering (two `poll:<n>` could reach cm inverted) and of a
deterministic AC3.

**No beat is ever retried.** No queue, no replay: a missed beat is lost and the next one
arrives in 5 minutes. That is the right trade for a liveness signal — replaying a stale
*"I am alive"* is at best useless, at worst a dated lie.

**The auth reuses `delivery_token` but the beat does NOT feed the auth-boundary ledger**
(D6). That population counts **report-delivery** failures — on the order of 4
attempts/day — and pouring **288 attempts/day** into it would change what the operator
query `… WHERE tool_name = 'auth_boundary'` measures and drown the signal mika#1949
exists to raise. The class information is not lost: it rides the transition WARN.

**`no_dispatch_test.rs` forbids the literal `"POST"` in every file of the module**, so
the sink writes `client.post(url)` — a `Method::POST` or string construction would carry
it and redden the LECTURE-SEULE guard. The contract itself is untouched: what is added is
an **outbound liveness** side effect, the same family as the report `POST` that has
existed since day one — no forge write, no ticket mutation.

**Guards.** `mika1990_le_battement_a_un_seul_ecrivain` — a source scan asserting **one**
composition site for `reason` and **one** call site to the sink in production, allowlist
shipped **empty**, with its anti-vacuity assertion (it reddens if the name it looks for
is written nowhere — a scan aiming at a dead name verifies zero things and reads exactly
like a clean tree, class mika#2205) and its good-faith control
`mika1990_le_scan_du_battement_voit_un_second_site`. No behavioural test can see that
class: a second beat writer makes **no decision wrong** the day it is written — the cycle
keeps running, every assertion stays green, and only the freshness becomes uncountable
(two interleaved `<n>` runs at cm). **When it fires, remove the second site — do not
allowlist it** (doctrine mika#2201).

**Egress.** Declared as `control-monitor-liveness` in `docs/egress/egress-manifest.toml`,
`logged = true` at field granularity: a **successful** beat writes nothing (AC4), but the
two **transition** lines carry `reason` and the milestone reference.

Operator surfaces (`manager_delivery_resolved.liveness_url_set`,
`manager_liveness_failed`, `manager_liveness_recovered`), expected regimes, the four
probes and their halts: root `CLAUDE.md` § *Optional (battement de vivacité du manager)*.

### Un vol est deux lignes de `tasks` (mika#2886)

L'évaluateur de vols (`milestone_manager/evaluator.rs`, mika#2698) classe l'issue
d'un vol de pilote en lisant une ligne `Outcome: <valeur> — <url>` dans le
`result` de la tâche. **Cette ligne ne vit pas sur la ligne que l'URL du ticket
désigne.**

| ligne de `tasks` | label | `reference_url` | `result` | ce qu'elle porte |
|---|---|---|---|---|
| parente | `ready-label: <owner/repo>#<n>` | l'URL de l'issue | **vide** | l'agent, les tours (métadonnées), la borne temporelle |
| enfant | `long_running:run_claude_pilot` | **NULL** | `Outcome: PR_OPENED — …/pull/<n>` | l'issue du vol |

C'est la topologie que `skills::executor::build_callback_task` produit, et elle
n'est pas réconciliable en aval : `try_extract_callback_metadata` ne remonte dans
la parente que des **champs nommés** de métadonnées, jamais le texte du `result`.

**La mesure.** `Database::list_finished_implement_flights_in_container` ne lisait
que la parente. `classify_issue` ne voyait donc jamais de ligne `Outcome:` et
rendait `DeathCause::Unknown("no_outcome_line")` → `ManualResumption`. Entre le
redémarrage f14e69d9 (2026-10-19 06:00:32Z) et le 2026-10-23, le puits a produit
**531 verdicts, tous identiques** : *« mort — cause non classée
(no_outcome_line) — reprise à la main »*. Le rapport de 08:04:26Z classait ainsi
le vol de #2875 « mort, 121 tours » — ce vol avait ouvert #2881, mergée à
08:01:18Z. Un instrument constant ne mesure rien, et c'est ce qui rendait le
compte de concordance de mika#2698 vol 2 (`MIKA_MANAGER_EVALUATE_WRITE`)
impossible : sa précondition est une concordance, pas une constante.

**Le correctif.** La lecture fait une **seconde requête** dans la même fermeture
`with_db` — la jointure de `find_dispatch_children_for_issue_url`, avec le
`WHERE` de la requête 1 déplacé sur `parent`, donc zéro paramètre de liaison de
plus et une requête pour tous les vols du cycle. Elle rend des
`FinishedFlightRow { flight, pilot_result }` : deux champs parce que les deux
données viennent de deux lignes. Écraser le `result` du `Task` rendu aurait
produit un type qui ment sur sa provenance. `pilot_result` est `None` — jamais
`Some("")` — quand aucun enfant ne parle (motif mika#2331), et la projection de
`flight_source.rs` est `pilot_result.or(flight.result)` : l'enfant d'abord, la
parente en repli pour le vol mort avant d'avoir produit un enfant.

Trois filtres, **tous en Rust et jamais en SQL**, pour la même raison que le
filtre de statut terminal de la requête 1 : la population
(`is_pilot_dispatch_label`, son lecteur unique — une clause `IN` recopiant les
labels serait une seconde source de vérité, et `long_running:build_mika` s'y
glisserait au premier élargissement), la vivacité (`is_terminal_task_status`) et
le contenu (un `result` présent **et** non vide, sans quoi un enfant muet
écraserait un enfant antérieur qui, lui, parlait). Le « dernier » enfant est
tranché sur `COALESCE(completed_at, updated_at)` puis sur l'`id`, sur l'ensemble
**déjà filtré** — un `ORDER BY … LIMIT 1` en SQL aurait choisi avant les filtres,
donc parfois un enfant muet.

**Aucune ligne de `evaluator.rs` n'a changé.** Le verdict ne change pas ; c'est
ce qu'il lit qui change. Les tests de mika#2792 semaient un vol dont la ligne
portant l'URL du ticket portait **aussi** l'`Outcome:` — la forme que la
production ne produit pas. C'est la leçon transférable : un test qui sème sa
fixture par le chemin le plus court sème une topologie, et celle-là n'était pas
la bonne. L'AC1 de mika#2886
(`flight_source::mika2886_la_forme_reelle_rend_un_verdict_de_pr_ouverte`) sème
désormais les deux lignes par les écrivains de production.

**Sonde post-déploiement.** `mika milestone reports --latest`, sur le premier
rapport du puits qui suit l'atterrissage d'un vol : un vol dont la PR est ouverte
ne doit plus être classé « mort ». **Halte** — un rapport encore constant en
`no_outcome_line` après un `make deploy` signifie que le binaire servi précède le
correctif (classe mika#2340), pas que la cause est ailleurs. Le compte de
concordance de mika#2698 vol 2 **repart de zéro** au redémarrage qui charge ce
correctif : les 531 verdicts antérieurs ne sont pas des mesures.

### Un vol relancé est **trois** lignes, et la parente est terminale au milieu (mika#2951)

La topologie ci-dessus décrit le vol `ready-label` classique. Depuis mika#1614
(« engine-side direct spawn, task reuse ») le moteur relance le pilote
d'implémentation **sur la row de la parente de groom**, et cette row est passée
`completed` à la fin du groom. Il y a donc une troisième ligne, et surtout :
**la terminalité de la parente n'est pas la terminalité du vol.**

| ligne de `tasks` | label | état à l'instant où la parente devient terminale |
|---|---|---|
| parente | `ready-label: <owner/repo>#<n>` | `completed`, `completed_at` = fin du **groom** |
| enfant 1 | `long_running:run_claude_pilot_groom` | terminal, `Outcome: PLAN_GROOMED` ou `NO_PR` |
| enfant 2 | `long_running:run_claude_pilot` | **créé, encore en vol** |

Le défaut que cela produisait : l'évaluateur retenait la parente dès
`is_terminal_task_status`, ne voyait que l'enfant de groom, et inscrivait son
issue au registre — où `write_and_swallow` la figeait pour toujours. Les trois
vols du 2026-10-28 ont ouvert les PR #2947, #2949 et #2950, toutes mergées, et
sont inscrits « mort PLAN_GROOMED · reprise à la main ». Sur ce chemin — c'est-à-dire
sur **tous** les vols depuis mika#1614 — le compte de concordance de mika#2698
ne pouvait pas monter.

**D1 — le discriminant est le label de l'enfant.** `run_claude_pilot` pour
l'implémentation, `run_claude_pilot_groom` pour le groom, constantes nommées
dans `task_engine::engine` (`IMPLEMENT_PILOT_DISPATCH_LABEL`,
`GROOM_PILOT_DISPATCH_LABEL`) avec `is_implement_pilot_label` pour lecteur.
Ni `dispatch_class` ni l'ordre ne pouvaient servir : la `dispatch_class` vit sur
la **parente** et c'est précisément elle que la relance réécrit de `groom` en
`implement`, donc elle ne distingue pas les deux enfants ; et l'ordre de création
est une propriété du dispatcheur, pas une donnée de la ligne — un re-groom
postérieur l'inverse. Le label, lui, a un producteur unique,
`skills::executor::build_callback_task` via `format!("long_running:{tool_name}")`,
et `mika2875_la_constante_est_la_forme_que_le_producteur_emet` lie chaque
constante à l'outil que ce producteur reçoit.

**D2 — la classe prime, jamais la récence.** `pilot_result =
dernier_implement.or(dernier_groom)`. « Le plus récent des enfants » est
exactement la moitié du défaut : un re-groom postérieur à l'implémentation
reprendrait la main et re-classerait un vol abouti en `PLAN_GROOMED`.

**D3 — la base rapporte, `flight_source` décide.**
`FinishedFlightRow::pending_implement` dit *qu'un enfant d'implémentation vole
encore*; il ne retire rien. Le filtre vit dans
`flight_source::finished_implement_flights`, qui partitionne et n'évalue que le
reste. Deux couches, parce qu'une base qui filtre rend deux populations sous un
seul nom et qu'elles ne sont pas soustractables (motif mika#2205) — et parce que
la sonde a besoin de voir ce qui a été écarté. `pending_implement` a un
**lecteur unique**, tenu par un scan de source à allowlist vide
(`mika2951_lajournement_a_un_lecteur_unique`) : deux fichiers de production le
touchent, `db/tasks.rs` qui rapporte et `flight_source.rs` qui décide.

**La borne temporelle porte sur la fin du VOL, pas sur celle de la parente** —
et c'est la moitié du correctif sans laquelle l'ajournement serait un oubli
définitif. Une parente passe la fenêtre quand sa propre terminalité **ou** celle
d'un de ses enfants est postérieure à `last_observed_at`. Sans cette
disjonction : le `completed_at` d'une parente de groom est figé à la fin du
groom, le poll tourne toutes les 5 minutes par défaut, donc le cycle qui vient
d'ajourner avance le point de reprise au-delà — et la parente ne repasse
**jamais**. AC1 tenu (aucun verdict prématuré), AC4 jamais (aucun verdict du
tout). La sous-requête ne filtre **pas** par label, délibérément : les filtres
fins restent en Rust derrière leur lecteur unique, et les deux erreurs ne
coûtent pas le même prix — surapprocher fait relire une parente que les filtres
Rust jugeront juste et que l'idempotence du registre protège, sous-approcher
perd le vol pour toujours. Épinglé par
`mika2951_une_parente_ajournee_repasse_la_borne_quand_son_implement_finit`, dont
les horodatages sont ceux du premier vol mesuré et dont le contrôle négatif
sépare « la disjonction fonctionne » de « la borne a été retirée ».

**Une prémisse extérieure, assertée plutôt que supposée.** Tout cela suppose que
l'enfant d'implémentation est créé *avant* que la parente devienne terminale.
C'est vrai parce que `deliver_callback` appelle
`try_complete_parent_on_callback_success` **avant**
`try_dispatch_pilot_after_groom_success`, et qu'un callback de groom ne porte pas
de `pr_url` — la première est donc un no-op sur cette parente. Si l'ordre
s'inversait, il y aurait une fenêtre où la parente est terminale sans enfant
d'implémentation, et elle serait jugée sur son groom : le défaut d'aujourd'hui,
revenu en silence. L'ordre est épinglé sur le source de `dispatcher.rs` par
`mika2951_la_parente_devient_terminale_apres_la_naissance_de_lenfant_dimplementation`.

**Le faux négatif qui reste, borné et assumé.** Entre la terminalité de la
parente et la naissance de l'enfant d'implémentation, `pending_implement` est
faux : la parente est rendue et le groom est lu, donc `not_a_flight` (le
classement de mika#2949). Ajourner sur la convergence du groom plutôt que sur
l'existence de l'enfant serait plus précis mais **non borné** — un groom convergé
dont la relance n'allume jamais (frein STOP, pas de jeton) resterait ajourné pour
toujours. On choisit le faux négatif borné contre le silence infini.

**Sonde.** INFO `flight_deferred_pending_implement` (`target:
"mika::milestone_manager"`), un par cycle, avec `deferred`, `task_ids` et
`scope = "container"`. `task_ids` est **absent** quand `deferred == 0`, jamais
`""` (motif mika#2331). L'**absence de la ligne** signifie un binaire antérieur
au correctif, pas un zéro (motif mika#2340).

**Les trois lignes déjà figées ne sont pas réécrites par le programme** : un
journal d'audit n'est pas une table que son écrivain corrige, et un rattrapage
automatique demanderait à `write_and_swallow` de savoir *quand* un verdict
antérieur fut prématuré — c'est-à-dire de réimplémenter l'AC1 à l'envers, dans le
seul site qui doit rester bête. Elles partent par un geste opérateur nommé,
borné aux trois `target_key` littéraux :
[`docs/operator/flight-verdict-premature-rows-2951.md`](../operator/flight-verdict-premature-rows-2951.md).
Le runbook dit aussi ce que la suppression ne fait **pas** : la concordance de
mika#2698 ne remonte pas de trois d'autant, elle cesse d'être *contredite*, ce
qui n'est pas la même chose.

**Env vars.** All optional with three-tier fallback: `MIKA_MANAGER_TARGET_MILESTONE` (Phase 1 single-target — loop disabled when unset), `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS` (default 21600 = 6h), `MIKA_MANAGER_POLL_INTERVAL_SECS` (default 300 = 5min; clamped to `min(poll, heartbeat)`), `MIKA_MANAGER_SILENCE_THRESHOLD_DAYS` (default 3), `MIKA_MANAGER_DELIVERY_URL` / `MIKA_MANAGER_DELIVERY_TOKEN`, `MIKA_MANAGER_ESCALATION_URL`, `MIKA_MANAGER_HEALTH_URL` (optional cm executor liveness endpoint), `MIKA_MANAGER_CHECKPOINT_DIR` (default `$HOME/.mika/manager/checkpoints`), `MIKA_MANAGER_OFFLINE_SINK_DIR` (default `$HOME/.mika/manager/sink`), `MIKA_MANAGER_LIVENESS_URL` (mika#1990, no default — absent ⇒ the beat channel is disarmed). **All eleven are declared in `.env.example` since mika#2267** — before it, not one `MIKA_MANAGER_*` variable was declared anywhere in the repo (`.env.example`, `docker-compose.yml`, `packaging/`), so no review had ever been in a position to notice their absence or their misspelling. `mika2267_every_manager_env_const_is_declared_in_env_example` keeps it that way.

**Cadence spawn.** `spawn.rs` (`manager_config_from_env` + `spawn_manager_cycle_task`, the latter taking an `Arc<dyn TokenResolver>` since mika#2013) wires the cycle as a background tokio task at `server::run_server` startup. Env-gated on `MIKA_MANAGER_TARGET_MILESTONE`: unset → INFO `manager_cadence_disabled` and no spawn; set-and-valid → INFO `manager_cadence_start` + spawn; set-and-malformed → ERROR `manager_cadence_config_invalid` and no spawn (startup continues). The loop polls at `cfg.poll_interval` and calls `run_manager_cycle`; the cycle body itself decides whether to deliver (state-change OR heartbeat-elapsed) and is a no-op otherwise. Cycle failures log at WARN and do not stop the loop — except a sustained authentication failure, which escalates per the token-renewal note below (mika#2013). Graceful shutdown responds to a dedicated `manager_shutdown_token` (sibling to `kg_shutdown_token` and `webhook_queue_shutdown`) — cancelled at the same `.with_graceful_shutdown` site. Structurally mirrors `kg::resolver_tick::spawn_resolver_tick_task` (same `tokio::select!` shape, same fail-open discipline, same `info!/warn!` lifecycle events). Injection-verified per `todos/mika-manager-cadence-wiring-injection-verification.md`.

**Token renewal + auth alarm (mika#2013).** The cycle token is re-resolved **before every cycle** through a `TokenResolver` (`SettingsTokenResolver` in production — PAT first per ADR-008, GitHub App installation token as fallback), never frozen at spawn. The founding bug: `ManagerConfig.github_token` was resolved once in `manager_config_from_env` and forwarded verbatim to `gh` forever, so an App installation token (~1h TTL) left the manager cycling 401 until the process restarted — 16 `manager_cycle_error auth_class=401` in one night with nothing louder than a WARN. The renewal machinery already lived in `GitHubApp::installation_token()` (memory cache + 5-min expiry buffer); the defect was only that it was asked once. A change in the resolved value emits `manager_token_refreshed` (INFO, presence booleans only — never token material). Paired with it, `AuthFailureTracker` counts the **duration** of an unbroken `AuthClass::Unauthorized` run (a duration, not a cycle count: `poll_interval` is operator-configurable, so N cycles has no stable temporal meaning). Past `AUTH_PERSISTENT_FAILURE_THRESHOLD` (30 min) it emits `manager_auth_persistent_failure` (ERROR) and escalates to `MIKA_MANAGER_ESCALATION_URL`, re-announcing at most hourly (`AUTH_ALARM_REEMIT_INTERVAL`). The escalation carries a dedicated `AuthAlarmBody`, **not** a `DeliveryBody` — the alarm fires because the milestone could not be read, so an `Assessment` there would be fabricated state on the wire. Any successful cycle clears the window; non-401 failures neither advance nor clear it (a network blip is not proof of recovery, nor of auth failure). A failed re-resolution KEEPS the previous token rather than overwriting it with `None` — `reader.rs` only sets `GH_TOKEN` when `Some`, so overwriting would silently drop the cycle onto the host's ambient credentials. The refresh is bounded by `TOKEN_REFRESH_TIMEOUT` (15s) because it sits outside the `select!` on the cancellation token and `GitHubApp` holds its cache write-lock across an un-timed HTTP call. `AuthClass::Forbidden` is excluded from the alarm — a known gap tracked as mika#2063, left open because AC3 of the ratified mika#2013 plan asserts 403 does not fire.

**Post-mika#1968 hardening (mika#1975).** Three changes, none of which moves a
delivery route or an alarm threshold.

*The boot probe is bounded.* `verify_gh_auth` wraps its single `gh` call in
`GH_AUTH_PROBE_TIMEOUT` (15s, a named constant, deliberately not
env-configurable — the same YAGNI rule `AUTH_PERSISTENT_FAILURE_THRESHOLD`
states). **What an unbounded probe blocked was never startup**, contrary to the
ticket: `spawn_manager_cycle_task` returns its handle immediately and
`run_server` never awaits it, while the probe runs *inside* the spawned task.
What it blocked is the **cadence loop before its first tick** —
`tokio::time::interval` is built after the probe — so a hung `gh` meant zero
cycles for ever, and it survived graceful shutdown too (`ProcessGhRunner::run`
has no timeout, and the `select!` on `cancel` is downstream of the probe, so
nothing drops the future `kill_on_drop` would need). The silence looked
healthy: `manager_cadence_start` and `manager_delivery_resolved` are both
emitted *before* the probe. New `AuthClass::Timeout` (`auth_class=timeout`),
posed on the `Err(Elapsed)` arm and **nowhere else** — `classify_cycle_error`
carries no `Timeout` pattern, and `"timed out"` stays under `Network` because a
transport timeout is an observation about the network while this variant is a
statement about a budget we enforced. Excluded from `is_auth_failure` for
`Network`'s reason: a slow host is not a closed door.

*The classifier stops reading its own arguments.* `ProcessGhRunner` now fails
with a typed `GhCommandError { args, stderr }` (house precedent: `DeliveryError`
+ `downcast_ref`, and the mika#2179 rule that error classes come from the
variant). `classify_cycle_error` / `classify_milestone_probe_error` take
`&anyhow::Error` and classify the **`stderr` alone**, falling open to the
rendered string when the downcast misses. The vector this closes is structural
rather than measured: the rendered error carried
`gh api /repos/{owner}/{repo}/milestones/{N}`, so with
`MIKA_MANAGER_TARGET_MILESTONE=owner/repo#403` *every* failure of that call —
a 500, an unrecognised DNS wording, a parse error — classified `Forbidden`,
reached `is_auth_failure` and fired the 30-minute alarm with the wrong class and
the wrong hint. The three numeric tests additionally go through
`has_http_status`, which requires a status marker (`http`/`status`/`code`)
within 24 characters upstream and non-digit boundaries. **No non-numeric pattern
was removed** — `unauthorized`, `bad credentials`, `gh auth login`,
`authentication token not found`: the last two are mika#2013's *missing-token*
shapes, which carry no HTTP status at all, and dropping one would make a
token-less manager classify `Other`, i.e. reopen the blindness mika#2013 closed.
That invariant (*the tightening removes false positives only*) is pinned by a
frozen corpus, `mika1975_the_classification_of_every_measured_auth_shape_is_unchanged`.
`stderr_head` deliberately keeps the **whole** rendered string, command line
included: the tightening is on what is *classified*, never on what is *logged*.

*The spawn guard cannot poison.* `MANAGER_SPAWN_GUARD` is an `AtomicBool` with
`swap(true, SeqCst)` instead of a `Mutex<bool>` + `.lock().unwrap()`. The ticket
asked for `PoisonError` recovery; the critical section was a `bool` read, a
`warn!` and a `bool` write, so **the only realistic panic site in it was the
logging macro** — a recovery path that logs is unreliable exactly when it is
needed. No poisoning test was added: asserting "it does not poison" on a type
with no poisoning is empty.

**Out of scope, named.** AC4 (proactive App-token refresh) is mika#2013's whole
subject and was removed from this ticket by an operator comment dated
2026-08-29; mika#2013 has since shipped it (`refresh_cycle_token`). The
cycle-body `gh` calls in `Reader::read` remain unbounded — same class, larger
blast radius (the loop freezes mid-cycle, in steady state), different remedy
(a per-call budget must compose with `poll_interval` and with
`MissedTickBehavior::Burst`), and a single global timeout in
`ProcessGhRunner::run` is refused because one constant cannot fit both
`gh api /milestones/N` and `gh pr list --limit 100 --search`, nor the CLI paths
where an operator is waiting. That is its own ticket.

**Phase 2 gates NOT wired.** Dispatch authority stays gated behind the three portes (forge-gate loop-résistance + contention exec + INTERNAL_TOKEN alignment) documented in the brief § 3. This module contains no dispatch class, no `run_claude_pilot` invocation site, no scope-approval callsite. Promotion to Phase 2 requires updating both `no_dispatch_test.rs` FORBIDDEN_TOKENS and the module docstring atomically.
