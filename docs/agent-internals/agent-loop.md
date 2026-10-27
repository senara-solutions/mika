# Agent Loop

Moved verbatim out of `crates/mika-agent/CLAUDE.md` on 2026-10-13. That file is loaded whole into a pilot's prompt as soon as the pilot reads anything under `crates/mika-agent/`; at 594 KB it no longer fit, and `crates/mika-common/tests/claude_md_size.rs` now caps every `CLAUDE.md` in the repository. Headings keep their original level.


Max 20 tool steps (all modes: conversation, callback, reminder, team) — **30 for mika-qa since mika#2926, see § Le plafond de pas est résolu par agent below** — a per-agent turn envelope **defaulting** to 5 minutes (settable since mika#2189 — see § Timeout Budgets), 30s default per-tool timeout (overridable via `Tool::timeout_secs()` for builtins, and by the owning skill's `timeout_secs` for skill tools — see § Per-Tool Skill Budget). `LoopMode::Silent { max_steps }` carries per-trigger step limits via `SilentTrigger::max_steps()`. Step-awareness nudge injected at step `max_steps - 2` for all modes to encourage wrapping up. Silent mode nudge text is tailored for `send_message` notification.

**Deadline enforcement (#848, #939, mika#2189):** the turn budget is enforced via an `Instant`-based deadline checked at five points: (1) top of each `run_loop` step iteration, (2) end of prelude work in each inner function before entering `run_loop`, (3) before `attempt_continuation_turn` entry — skip when `now + CONTINUATION_TIMEOUT_SECS > deadline`, (4) inside `attempt_continuation_turn` itself, where the inner timeout is clamped to `min(60s, deadline - now)`, (5) inside the LLM transport retry loop — `send_message_with_deadline()` aborts the retry chain when the remaining budget falls under a threshold **derived from the effective per-call plafond** (`0.75 × plafond + 0.25 × plafond`, which is the historical 90 + 30 = 120s at the default plafond), preventing doomed retries from consuming the deadline (#939). **The outer agent deadline still never drops a future mid-flight** — that is the mika#848 contract and it is unchanged: the `llm_calls` row is always persisted (success or transport-timeout), and worst-case turn duration remains `envelope + plafond`, i.e. `300s + 120s = 420s` at the shipped defaults. What changed with **mika#2342** is that the provider's `reqwest` timeout is no longer the **sole** cancellation mechanism: `run_loop`'s call is wrapped in a `tokio::time::timeout` sized on `LlmProvider::worst_case_failure_secs() + LLM_WATCHDOG_MARGIN_SECS` (60 s) — see § LLM-call watchdog below. Sized on the *worst case* rather than on the remaining deadline, that net does not contradict the mika#848 reasoning: on a rail that bounds its own calls it can only fire once the rail's mechanism has already failed, i.e. on a call that is already lost. Note the Anthropic rail (`crates/mika-common/src/claude.rs`) still hard-codes its own `120s` literal instead of reading the plafond — a real inconsistency mika#2189 names and leaves to its own ticket; mika#2342 makes it a *declared* value (`AnthropicProvider::worst_case_failure_secs`) so the net is not sized on a budget that rail does not honour. `LoopResult` is a three-variant enum (`Done`/`MaxStepsExceeded`/`DeadlineExceeded`) without `#[non_exhaustive]` — the compiler's match-exhaustiveness check enforces that all three outer handlers (conversation, silent, team) handle every variant. CI lint guard `scripts/check-loop-select.sh` rejects `tokio::select!` inside `run_loop`'s body (would shadow the iteration-top deadline check).

On max-steps exceeded: continuation turn (tools disabled, deadline-clamped timeout, ceiling 60s) forces a text summary via shared `attempt_continuation_turn()` helper (used by Conversation, Team, and Silent modes); the helper persists an `llm_calls` row in all outcomes (success/error/timeout) so the continuation is never the silent-drop variant of the in-flight-cancel bug at smaller scale. If continuation fails or is skipped (deadline too close), structured fallback shows last 5 tool names with status. Silent mode continuation sends the summary via `message_sender` if available, prefixed with "[Background task exceeded tool step limit]".

**Test-only entry points (`run_*_with_deadline`):** publicly visible by naming convention, used only by `EvalHarness` to inject short deadlines for the deadline-during-LLM-call eval scenarios. Production callers should always use `run_agent`/`run_silent_agent`/`run_team_agent`. `AgentParams` carries no deadline knob.

### Le plafond de pas est résolu par agent, et c'est celui-là qu'on annonce (mika#2926)

**Un site unique de résolution.** En tête de `run_loop` :

```rust
let max_steps =
    crate::planning::policy::max_tool_steps_for_agent(db.agent_id(), mode.max_steps());
for step in 0..max_steps { … }
```

`mode.max_steps()` fournit le **défaut** — `MAX_TOOL_STEPS`,
`MAX_CALLBACK_TOOL_STEPS` ou `MAX_TEAM_TOOL_STEPS` selon le `LoopMode`, parce que
c'est le mode qui sait lequel des trois s'applique — et la fonction ne le
remplace que pour `mika-qa`, à qui elle rend `MAX_TOOL_STEPS_MIKA_QA = 30`. Les
trois modes passent par ce site et rien en aval ne recalcule un plafond : **c'est
ce qui tient AC3 structurellement** plutôt que par relecture. La résolution est
sensible à la casse (`"Mika-QA"` reçoit le défaut) ; les deux propriétés sont
affirmées agent par agent dans `planning::policy`'s `mod tests`
(`mika2926_le_plafond_par_agent_ne_touche_que_mika_qa`,
`mika2926_la_resolution_est_sensible_a_la_casse`).

**Pourquoi 30, et pas 25 ni 40.** Mesuré sur les 13 tours de mika-qa coupés au
plafond entre le 2026-10-23 et le 2026-10-26 (série `max_steps_tool_census`,
mika#2877) : `run_shell` consommait de 9 à 28 pas, et le `total_calls` maximum
observé était 31. 30 couvre 12 des 13 tours ; le treizième est accepté comme
coupé, parce qu'un plafond dimensionné sur le pire cas observé n'est plus un
plafond. La borne d'exploration posée dans les deux prompts de revue est l'autre
moitié du correctif — le relèvement seul **déplace** l'épuisement, la mesure le
disait avant le geste.

**Le plafond SERVI est transporté, jamais recalculé (R1b).**
`LoopResult::MaxStepsExceeded` porte un champ `max_steps: usize`, rempli depuis
le local déjà résolu. Trois consommateurs en aval le lisent là, et chacun
annonçait auparavant un nombre qui pouvait divergir :

| consommateur | ce qu'il lisait avant | pourquoi c'était faux |
|---|---|---|
| WARN `"silent agent exceeded max tool steps"` | `params.trigger.max_steps()` | le déclencheur **déclare** 20 pour un `Callback` ; c'est le plafond du déclencheur, pas celui que la boucle a servi |
| `persist_deadline_fallback` (bras conversationnel) | la constante `MAX_TOOL_STEPS` | un quatrième site annonçant un plafond, non nommé par le ticket, trouvé en vol — c'est le site que le tour `synchronize` 4f94fbc8 de mika#2920 a traversé |
| `max_steps_tool_census` | le local (déjà correct) | inchangé — il est émis **dans** le corps de boucle |

L'invariant à retenir : **un nombre de pas annoncé hors de `run_loop` se lit sur
la variante, jamais sur une constante ni sur le déclencheur.** Le bras `Team`
détruisait déjà la variante avec `..` et n'a rien demandé.

**Et le prompt est le quatrième annonceur.** `qa-review/system_prompt.md` dit
*« You have a maximum of N tool steps per turn. »* ; N vivait dans un fichier que
rien ne joignait au moteur, et valait **14** pendant que le moteur servait 20.
L'égalité est désormais une assertion :
`tests/qa_review_executes_repo_guards.rs § mika2926_le_plafond_annonce_par_le_prompt_est_celui_du_moteur`
extrait N du prompt et le compare à ce que `max_tool_steps_for_agent` résout — la
fonction, jamais un littéral recopié. Les deux nombres doivent donc bouger dans
le même commit.

**Attestation de bout en bout** : `tests/eval/test_qa_step_budget_2926.rs`, trois
tours sur le **même** corpus rejoué (la session `callback-0f07c520`, 23
`run_shell` puis la revue) — mika-qa pose son verdict sans census, un autre agent
épuise à 20 sur la même séquence (le contrôle négatif, qui est l'information
disant que le changement est bien par agent), et le WARN de coupure annonce 30 et
non 20. Le troisième passe par `run_silent_agent` monté à la main, parce que
`EvalHarness::run` appelle `run_agent` et que `callback_turn(true)` ne pose qu'un
drapeau sur `AgentParams`.

### Timeout Budgets (mika#2189)

**The asymmetry this closed.** Two numbers govern a turn: the **per-call
plafond** (what `reqwest` is given) and the **per-agent envelope** (the turn
deadline). Since mika#1660 an operator could raise the plafond via
`MIKA_LLM_HTTP_TIMEOUT_SECS`; the envelope was a bare `300` constant in
`planning/policy.rs` with no env var and no per-agent setting. **One could raise
the ceiling and not the room meant to contain it** — and raising the plafond
alone lets a single call eat the whole envelope of a pass that averages three.

The founding measurement (7 days ending 2026-09-05, `llm_calls`): **209
failures** under one message — `failed to read response body: … operation timed
out` — whose latency distribution has no tail, only two values: **240 s** (171
occurrences) and **120 s** (37). Not provider variance; a client guillotine
crossed once or twice. Three agents, two models, one provider — so a model
fallback for mika-arch would have moved 7 % of the problem.

**`LlmTimeoutBudget` (`mika-common::llm::budget`)** now holds the pair, and the
three retry thresholds are **derived** from the plafond rather than written
beside it: `typical_call = 0.75 ×`, `retry_buffer = 0.25 ×`,
`transport_retry_min_remaining = 0.50 ×`. Those fractions reproduce the old
literals (90 / 30 / 60) **exactly** at the 120 s default, so the migration is a
no-op at the shipped geometry and follows any later setting — where literals
would have silently described a geometry that stopped existing.

**Containment invariant (D2).** `LlmTimeoutBudget::validate` refuses
`plafond >= envelope` at **provider construction**, the same lifecycle point
where mika#1660 already panics on a too-small plafond. Consequence, written down
rather than discovered: **a `mika` that boots is not proof that its budgets are
valid; the first call is.**

**Bounded failure cost (AC3-b).** Raising a plafond cannot slow a call that
already succeeded. The regression it *does* create is that a **failing** call
gets more expensive, so `max_attempts = floor(envelope / plafond)` (clamped to
the provider's own `MAX_RETRIES + 1`) makes `attempts × plafond ≤ envelope` true
by construction. At the default geometry that is **2** — exactly what the
measurement shows. Not a pure no-op, and saying otherwise would be more
comfortable and less true: before this, a third attempt could start at exactly
`remaining == threshold` and carry a failure to 360 s, past the 300 s envelope.

**Reading the envelope.** `planning::policy::agent_total_timeout_secs(llm)` takes
it off the **provider**, which already holds the plafond — taking both from one
object is what stops them drifting apart again. The retired constants
(`AGENT_TOTAL_TIMEOUT_SECS` and its team sibling, which was a second literal
`300` whose doc comment merely *promised* it matched) are gated by
`policy::tests::no_bare_agent_timeout_constant_remains`, a source scan over
`src/`. **One documented exception:** `DELEGATE_TASK_TOOL_TIMEOUT_SECS` stays
static because `Tool::timeout_secs` takes `&self` and cannot reach a provider —
an agent with a raised envelope delegating through that tool would be cut at the
default. Not reachable today (mika-arch has no `delegate_task`), so it ships as a
stated bound rather than a `Tool` trait change bundled into a timeout ticket.

**Per-agent settings** (`~/.mika/agents/<name>/config.toml`, next to
`llm_provider` / `openrouter_model`): `llm_http_timeout_secs` and
`agent_total_timeout_secs`. The fleet stays at 120/300; **mika-arch is retuned to
240/900** (`MIKA_ARCH_CONFIG`), derived from its measured p99 = 191 s, max =
233 s and 3.1 calls per pass. Deliberately **no per-family default** (Q3): a new
slow agent discovering the problem through a failure is an accepted, *visible*
risk — the invariant's message names both values and the file to fix.

**Post-deploy check (V3).** Re-run the failure-latency count per agent after at
least 24 h and 100 mika-arch calls. The criterion is that 120 s / 240 s vanish
from the latency column of errors. **If a fresh signature appears at 240 s /
480 s, halt** — the plafond moved and so did the distribution, which says the
model of the failure is wrong, not that the value is too low. Capture the
baseline before deploying: `prune_old_llm_calls` purges old rows.

**Observability + boot guard (mika#2293).** mika#2189 made the pair settable and
left it unobservable, and the two failures that follow from that are closed here.

*The pair is now said out loud.* `llm_budget_resolved` (INFO, **ungated** by
`MIKA_STORE_LLM_CALLS`) is emitted at `server::init_agent` and per team run in
`teams::engine` — the two callers that already hold the agent's name, rather than
widening `create_provider_with_budget`, a free function that knows neither agent
nor home. It carries `agent_id`, both values, `max_attempts`,
`worst_case_failure_secs`, and the **provenance** of each half. That last field is
the point: `agent_config` at 240 means the setting is in force and the cause of a
cut is elsewhere; `process_env` means a fleet-wide variable is shadowing the
per-agent file; `default` means the file was never read or never carried the key.
Three remedies, and none of them is deducible from a plafond someone raised — which
is why mika-arch's `240/900`, shipped 2026-09-06, could fail in silence until the
2026-09-11 measurement. Deduplicated on the resolved pair, so a repetition is silent
and a **change** is re-emitted. **Deliberate blind spot, written at the emission
site:** the per-skill `[llm]` override path (`agent_loop`'s `make_provider_for`,
one provider per override) emits nothing — the question is an agent's *nominal*
budget, not what a skill overrides for one turn. Reader lives in
`mika_common::llm::budget_provenance` (see `mika-common/CLAUDE.md` for why the
cascade is rebuilt rather than recorded, and why its inverted order is pinned).

*The pair is now readable on demand, and the freeze is the property (mika#2457).*
`AgentState.budget_record` holds the `ResolvedBudgetRecord` this agent was
initialized under — resolved **once** at `init_agent`, from the same call that
emits `llm_budget_resolved`, and never recalculated. Same *not hot-swappable*
contract as `tier` (mika#1962) and `deployment` (mika#2290), for the same reason:
it reports the state the agent **runs under**, not what the disk carries now.
`GET /api/v1/agents/{id}/budget` (dashboard-or-internal auth, like its
`/sessions` and `/audit` neighbours) serves it verbatim, `resolved_at` included.

**It does not re-read the disk, and that refusal is the design.** Recomputing at
request time would walk the cascade in *this* process and could name a setting
that is not in force — mika#2304's defect one field over. It is also what keeps
the route and the disk **two distinct facts**, whose difference *is* the drift
measurement mika#2457 asks for: the operator compares `resolved_at` against the
`config.toml` mtime, and a route that refreshed itself would erase the
comparison. A **404** means "this server has not resolved that agent", never a
default — the lookup reads the resolved-agent map rather than `resolve_agent`,
which would lazily construct an agent (DB, skills, task engine, KG) as a side
effect of a read-only GET. `mika agents budget` renders that 404 as *"not
attested"* and prints **no** locally computed value; the CLI-side structural
guard is in `crates/mika-cli/CLAUDE.md`.

*And the drift, frozen with it (mika#2473).* `AgentState.model_drift` is
established at the same `init_agent`, from the same record, and served as a
**sibling** of it: `{ budget, model_drift }`. A sibling rather than a field,
because `ResolvedBudgetRecord` keeps its single constructor and its dedup
signature untouched — the scan that refuses a second constructor stays green with
an empty allowlist. `init_agent` compares what `well_known_agents.rs` declares for
this agent against what the cascade resolved, emits `well_known_model_drift` (WARN)
or `well_known_model_in_sync` (INFO) once, and takes the boot note D2 needs. **No
new error path**: nothing here refuses a boot. **Two** of the three agents on a
dev workstation are in measured drift today (mika-arch `kimi-k2.5`→`kimi-k3`,
mika-qa `zai`→`openrouter`); the third, mika-dev, is in phase — and reporting all
three, whichever the verdict, *is* the deliverable — muting that
population with an allowlist would rebuild the silent guard mika#2328 measured.

*And once per turn, whether the disk moved under the process.* The per-turn half
sits at the head of `load_agent_context` — the **single funnel** of all three
loops, team runs included, which is why `mika2473_the_freshness_check_sits_in_the_one_funnel`
is a source scan asserting exactly one call site and shipping no allowlist: two
sites would share one dedup key and silence each other. **That scan walks the
whole crate**, not `agent_loop/mod.rs` alone: it shipped reading one file, which
left it green on a second call site in `server/`, `teams/` or `task_engine/` —
i.e. blind exactly where a stranger to this contract would write, and reading as
coverage while covering nothing (measured: a `detect_config_change` planted in
`init_agent` kept the one-file form green). Its companion control
(`…_catches_a_second_site`) keeps the scan from going vacuous and now injects its
second site in a **different** file. A turn whose agent this process never
initialized finds no boot note and triggers nothing — that, and not "team runs",
is the exempt population.

*Nothing on that per-turn path may panic, refuse, or shout every turn.* Three
properties the review of mika#2473 had to close, all in
`mika-common::llm::config_freshness`. (a) The mtime is formatted by a **fallible**
conversion with a named fallback (`CONFIG_MTIME_UNREPRESENTABLE`);
`DateTime::<Utc>::from(SystemTime)` ends in an `.unwrap()` and a filesystem will
store an instant outside chrono's range, so the shipped form could panic on a
turn — and it wrote `reported` *before* converting, which marked the drift
reported and lost it for good. (b) The global `BOOT_NOTES` mutex is **not held
across the `stat` nor across `resolve_llm_budget_record`** (itself up to three
more synchronous reads): it is shared by every agent, so one slow `config.toml`
serialised every other agent's turn. Releasing it opens a race, closed by
re-checking `Reported::covers` under the second acquisition, so R9's
at-most-once-per-distinct-mtime still holds. (c) `agent_config_mtime_unreadable`
is emitted on the **onset and on the recovery**, not once per turn — the readable
path was deduplicated and this one was not, so a `config.toml` made unreadable
produced one WARN per turn for ever on an arm whose expected regime is zero
(mika#2131). The population semantics are untouched: an unreadable `stat` still
leaves the population and still leaves `BootNote::reported` intact.

*`restart_required` answers "a value in service differs", not "the record
differs".* `ResolvedBudgetRecord` carries `*_source` and `*_raw` provenance, so
moving a setting between cascade doors **at the same value** used to raise the
loud arm and WARN that a restart was needed — a false alarm about a change a
restart would not apply. The comparison now clears dating *and* provenance on
both clones, and a door-only move lands on the INFO arm, whose wording already
says exactly that.

*The D1 assembly is a function, so it can be tested as one.*
`server::establish_model_drift(agent, global_home, agent_home, &record)` carries
`find_well_known_agent → config_toml → declared_model → compare →
emit_model_drift → note_config_at_boot` and returns the check. Inline, the
sequence was only ever exercised through its primitives: swapping
`global_home`/`agent_home` at the note, or dropping a step, compiled and passed
every test while D2 watched a file nobody edits.
`mika2473_the_boot_assembly_returns_the_check_and_takes_the_note` asserts the
verdict, the note under the right key, **and** that D2 then sees an edit to *that
agent's* `config.toml` — the only term that catches the swap.

*The D1 line says whether the declared model was written or defaulted.*
`declared_model_is_provider_default` rides both comparing arms of
`ModelDriftCheck` onto `well_known_model_drift` / `well_known_model_in_sync`. It
was computed and never read; on the line it is actionable — reconciling the
constant means **writing** a model key when nothing declared one and **editing**
one when something did, and on the in-sync arm it says the agreement rests on a
provider default the provider can move without any file changing.

*A half-configured pair now fails at boot.* `server::budget_guard::assert_llm_budgets_valid`
runs in `run_server` after `provision_well_known_agents` (which writes the
`config.toml` carrying the pair) and before any agent is initialized, over the same
`servable_agent_names` population as `tier_guard`. It refuses startup naming the
agent, both values, their provenance and the key to fix. **The failure it closes:**
`MIKA_LLM_HTTP_TIMEOUT_SECS=300` on the service without raising the envelope gives
mika-dev and mika-qa `cap = 300 >= envelope = 300` — no LLM call ever again — while
**mika-arch survives** on its own `config.toml`'s 900. Two agents silent, a third
answering: the shape that looks least like a configuration mistake and gets blamed
on the provider most readily, which is the misreading mika#2293 exists to correct.
Pre-existing violations get **no grace period and that costs nothing** — such an
agent could not make an LLM call anyway, so the guard moves an existing failure from
the first call to startup and makes it legible. It corrects no value and invents
none. It also **precedes** mika#1660's panic rather than replacing it: the `None`
path of the plafond goes through `llm::http_timeout_secs()`, which aborts on an
unparseable or below-floor value without naming the agent or the cascade door, so
the guard reads the raw values through the non-panicking `BudgetProvenance` the
observability half already writes. The panic stays for unguarded paths — a `mika`
CLI reaching it still panics exactly as before.

### LLM-call watchdog (mika#2342)

**The failure.** A mika-arch turn entered `llm_call started` and returned
**nothing for 27+ minutes**: no `llm_call completed`, no `turn_usage`, no error.
The task stayed `in_progress`, the A2A client walked away, the groom delivered
`first-pass _arch_ask failed`. Twice, on the same brief, with
`MIKA_LLM_HTTP_TIMEOUT_SECS=420` and `MIKA_AGENT_TOTAL_TIMEOUT_SECS=600` in the
service environment — a call of that length *should not exist*.

**The structural defect, which is readable without reproducing the incident.**
`run_loop`'s call was made bare, and the envelope is a test at the **top of the
iteration**, not a wrapper. A call that does not return the future never reaches
that test. So the provider's `reqwest` timeout was the **sole** mechanism able to
end an in-flight call, and when it failed — for a reason still unnamed — nothing
in the process could bound it and nothing recorded that anything had happened.
The asymmetry made it hard to read as a choice: `attempt_continuation_turn`, in
the same file, has wrapped *its* call in a `tokio::time::timeout` all along and
persists a row on all three arms.

**The net.** `tokio::time::timeout(llm.worst_case_failure_secs() +
LLM_WATCHDOG_MARGIN_SECS, …)`. On timeout: a `llm_call_watchdog_fired` WARN, an
`audit_events` row (`tool_name = 'llm_call_watchdog'`), and a synthetic
`LlmError::Transport` routed into the **pre-existing** `Err` branch, so the
`llm_calls` row is persisted with `status = "error"`, the real latency and
`request_bytes` without duplicating the persistence path.

**Why the worst case and not the remaining deadline.** Cutting on `remaining`
would re-open mika#848: a call dropped in flight loses its result *and* its row,
including when it was about to succeed. Sized on the worst case, the net fires
only after the rail's own mechanism has already failed — on a call that is
already lost, so cutting it costs nothing and its firing is first-order
information. **That property is conditional and the code says so:** it holds
*because* the three production rails bound their own calls, not because the
`LlmProvider` trait requires it. On a rail without a transport timeout (the mock
today, a new rail tomorrow) the net is the *first* mechanism, not the second.

**The worst case is declared by the rail, never derived from the budget.**
`LlmProvider::worst_case_failure_secs()` defaults to
`timeout_budget().worst_case_failure_secs(DEFAULT_ATTEMPTS_HARD_CAP)`, and
`AnthropicProvider` overrides it: that rail hands `reqwest` a `120s` literal
instead of the plafond, so at `MIN_HTTP_TIMEOUT_SECS` a derived value would read
40 s against a transport that can take 480 — a guaranteed false positive. At the
current geometries: 420/600 → `max_attempts = 1`, net at **480 s**; 240/900 →
`max_attempts = 3`, net at **780 s**. Both far under the 27 minutes observed and
above any legitimate chain.

**Per-attempt instrumentation is the discriminator, not decoration.** The net
alone says "something exceeded the worst case" without saying what.
`llm_call_attempt` (before each `send_once`, on the three rails, carrying
`attempt`, `max_attempts`, `request_bytes`) separates three readings of the same
silence: N attempts of roughly a plafond each (the chain runs, the cause is
upstream); one attempt outliving the plafond (reqwest did not bound it); no
attempt at all after `started` (the block is *before* `send_once` — body
serialization, connection acquisition). `request_bytes` rides on the **start** of
the attempt because that is the only way to know the size of a request that never
comes back: `llm_calls.request_bytes` is written after the call returns.

**Structural guard.** `agent_loop::tests::mika2342_every_llm_call_is_wrapped_in_a_timeout`
— a source scan, because removing the net breaks no assertion: the loop keeps
working and every existing test stays green, only the call goes unbounded and
silent again. Inventory closed at two call sites, **no allowlist** (an allowlist
born empty is a place to put the next violation); a third site is halt-and-surface.

**Operator surfaces.** `llm_call_watchdog_fired` in `$MIKA_SPIRIT_LOG_FILE`
should stay **empty** — any hit is a call reqwest failed to bound, i.e. the root
cause of mika#2342 made visible, and it feeds the follow-up ticket.
`SELECT COUNT(*) FROM audit_events WHERE tool_name = 'llm_call_watchdog';`.
`grep llm_call_attempt … | jq 'select(.attempt > 0)'` gives the real retry volume,
until now uncountable. **Halt condition:** repeated `llm_call_watchdog_fired` on
one agent with no intervening `llm_call_attempt` points at a block *before*
`send_once` — a different defect; do not raise the net.

Tool call summaries (name, truncated input/output, success, non_zero_exit) persisted in `messages.metadata` JSON column for cross-turn introspection (capped at `TOOL_METADATA_MAX = 4000` chars — tail entries dropped when exceeded, #744). The `tool_calls` DB table is the authoritative source; the dashboard's inline `ToolCallsTable` fetches from `GET /api/v1/traces/:trace_id/tool-calls` with metadata as fallback for pre-v15 messages. `MessageResponse` exposes `trace_id: Option<String>` to enable this lookup. `non_zero_exit` is set by heuristic detection of `Exit code:` / `Killed by signal:` prefixes from exec handlers; history builder tags these with `[NON-ZERO]` (distinct from `[FAILED]`). History builder appends `<context type="tool_history">` blocks to assistant messages.

Compaction includes tool names in summarization. Multi-modal tool results: `ToolOutput` carries optional `images: Vec<ImageData>` (base64-encoded), converted to multi-block `tool_result` content arrays for the Claude API. Prior-turn images are stripped before each API call to prevent unbounded memory growth.

**Per-turn tool_use dedup guard (#582):** `process_tool_calls()` deduplicates identical `(tool_name, arguments)` pairs emitted inside a single LLM response. The underlying tool runs once, the `tool_calls` DB row is saved once, one `ToolCallSummary` is emitted, and duplicate tool_use ids receive a `tool_result` built from the cached `ToolOutput` so the conversation/API history stays paired. Images on the cached result are stripped before reuse so the duplicate does not re-consume the shared `image_bytes_budget` (the LLM already received the images on the first duplicate's `tool_result`). Defends against provider-side duplication (observed with non-Anthropic providers). Logs `warn!` with `trace_id`, `tool`, `step`, and `cached_was_error` when it fires.

### Post-Conditions (EndTurn Chain)

Twelve sequential post-conditions on assistant text responses, plus one early-accept:

1. **Text-based tool call detection:** `detect_text_based_tool_call()` catches XML-style patterns (`<function=...>`) that slip through `extract_xml_tool_calls()` in mika-common, re-prompts the LLM once.
2. **Prose-style tool call detection (#569):** `detect_prose_style_tool_call()` catches function-call-style prose patterns (`tool_name({"key": "val"})`) where the identifier matches a registered tool (builtins + skills + MCP). Gated against the tool set to avoid false positives on code examples. Single retry.
3. **Required-tools gate:** When keyword-matched skills declare `[constraints] required_tools`, the engine tracks tool calls across all steps; if required tools haven't been called, the response is rejected once. `filter_available_required_tools()` pre-filters against builtins + skill tools + MCP. Only `Keyword`-matched skills contribute constraints (#463). **Self-contained response instruction (#890):** The correction message includes a persistence-awareness clause instructing the LLM to restate the full content on its corrected response — not reference prior turns — because only the final `EndTurn` is persisted to `messages`. Defense-in-depth: mika-arch skill prompts (`mika-arch-groom-ticket`, `mika-arch-second-review`) reinforce the same contract in their `### Constraints` section. **Terminal failure bypass (#516):** `has_terminal_required_tool_failure()` checks `all_tool_summaries` for required tools that failed with known terminal errors (GitHub self-approval, HTTP 4xx, permission errors). When detected, the gate allows EndTurn without retry — the agent attempted the tool and hit an unrecoverable wall. `is_terminal_tool_error()` classifies output via `RETRYABLE_ERROR_PATTERNS` (checked first, takes priority) and `TERMINAL_ERROR_PATTERNS`. Unknown errors default to retryable (conservative).
3b. **PR review early-accept (#695, #821):** `has_successful_pr_review()` checks if `all_tool_summaries` contains a successful `run_gh` call with `"pr"` and `"review"` in the input. When true, guards #3 (required-tools, #821), #4–#6b, #7–#9 are all skipped — but NOT guards 6c (asserted_unavailability) or 6d (assert-grounded) (#1178). Those two detect claim-without-evidence, orthogonal to the PR-review completion semantics. — the qa-review workflow's primary action completed and forced continuation would risk duplicate submissions. Defense-in-depth (two layers): (1) Session-scoped `pr_reviews_posted` map on `AppState` (`DashMap<String, HashSet<String>>`, #821) prevents duplicate reviews across turns within the same session — the primary defense, keyed by `(session_id, repo|pr_identifier)`. Entries evicted at the 5 dispatch/delegate session-teardown sites (`delegate_task.rs` + 4 `dispatcher.rs` callsites). Conversation-mode sessions that post a review are not covered — a known slow, bounded leak (tracked as coherence-debt DEBT-E in `mika-platform/docs/coherence-debt.md`). (2) Per-turn `ToolContext.pr_review_posted` AtomicBool (#695) rejects duplicates within a single turn. Both guards reject `pr review` calls with structured `duplicate_pr_review` error. `make_pr_dedup_key()` derives the session-scope key from `gh pr review` arguments.
4. **Completion-claim guard (#483):** `detect_completion_claim()` detects completion-claim keywords (`merged`, `deployed`, `complete`/`completed`, `shipped`) in assistant text. If detected AND `update_task_status` is in the tool registry AND it was not called AND active tasks exist, the response is rejected once. Skips for delegates and team agents.
4b. **Milestone-close-claim guard (#797, #1207):** `detect_milestone_close_claim_without_patch()` detects when assistant text contains a first-person claim that a GitHub milestone was closed (regex `\b(I|we|i've|we've)\s+(closed|closed out|completed)\b.{0,40}\bmilestone\b`) without a satisfying `run_gh` call in the turn. A qualifying call has `input_summary` containing `"api"`, `"PATCH"`, `state=closed`, and a milestones API path. Discrimination granularity (#1207): when the claim contains a parseable milestone number, suppress only if that specific number appears in a PATCH URL within the turn; otherwise fall back to presence/absence. This is a deliberate divergence from #483's presence/absence pattern, justified by mika-arch's multi-milestone review surface — a single turn may legitimately PATCH one milestone while writing prose about another. Single retry tracked via standalone `milestone_close_claim_retry_done` flag. Composes with #4: when both regexes match (e.g., "I completed milestone#14"), #4 fires first; #4b fires on a subsequent EndTurn after #4 is satisfied. After the retry, second-violation in the same `run_loop` emits a `warn!` and accepts the EndTurn (single-retry budget exhausted).
5. **Fabricated action-claim guard (#308):** `detect_fabricated_action_claim()` detects when the agent claims to have performed an action with a GitHub resource URL but made zero tool calls in the turn. Single retry.
5b. **Dev-groom fabrication guard (#1133):** Detects `Verdict: GROOMED` or `Verdict: ESCALATE` in conversation-mode assistant text without a successful `run_claude_pilot_groom` call in the turn. dev-groom is a dispatcher — verdicts arrive via callback from claude-pilot, not from the dispatcher LLM's turn. Gated: only fires when `run_claude_pilot_groom` is in `enabled_tool_names` (the dev-groom skill is loaded for the agent — producer skills like mika-arch-second-review that legitimately emit Verdict lines don't have this tool and bypass the guard entirely). Conversation mode only (`mode.is_conversation()`); callback turns (Silent mode) legitimately carry Verdict lines from the inner session and bypass this guard. Single retry with corrective re-prompt. Companion manifest fix: `dev-groom/skill.toml` was changed from `[output] required_suffix_lines` (producer shape) to `[constraints] required_tools = ["run_claude_pilot_groom"]` (dispatcher shape). Static parity test `test_dispatcher_skills_dont_declare_required_suffix_lines` guards against regression.
5c. **Distribution Doctrine public-promo guard (mika#1814):** Detects assistant text that proposes, drafts, or plans a prohibited public-launch surface — Show HN, Product Hunt, Reddit launch thread, Twitter promo thread, growth-hack — in violation of Mika's invitation-only distribution invariant. Two-layer regex: Layer A (subject) matches one of the prohibited-surface keywords; Layer B (verb) matches a first-person/second-person proposal or drafting verb (bilingual FR + EN — the founding incident, Al B 2026-07-20, was in French; Vincent operates on operator-tier in English). Both layers must fire so an educational answer ("Mika does not do Show HN — she grows by invitation") does NOT trigger the guard. Applies uniformly across modes (conversation, silent, team) and both tiers (family/operator) — a heartbeat that spontaneously drafts a Show HN would be exactly as bad as a conversation-mode one. Single retry via `intent_guard_retries` with label `DOCTRINE_PUBLIC_PROMO_LABEL`. Not skipped by `skip_remaining_guards` (#1178) — a successful PR review does not grant license to draft a public launch. Structural half of the mika#1814 fix (the prompt-template `## Distribution Doctrine` section is the intent half, code-managed in `prompt::write_distribution_doctrine_section`; see `feedback_prompt_enforcement_fragile`). Position: after 5b (dev-groom fabrication), before 6 (intent-precondition registry) — same fabrication-class family. Related: mika#1798 (umbrella non-transit doctrine bake), mika#1783 (substrate leak "Salut Vincent").
5d. **False local-hosting claim guard (mika#2290):** Refuses assistant text that asserts **this instance** runs locally — or that the user's data never leaves their machine — while the resolved `Deployment` is not `Local`. Founding incident: a cloud tenant answered « tout tourne en local, tes données ne quittent pas ta machine » to a campaign guest on 2026-09-11. Position 5d, immediately after 5c, whose form, `intent_guard_retries` budget and `guard.*` telemetry it reuses; same fabrication family, one step more self-referential.

**What it closes, and what it does not need.** The prompt half (the `## Runtime` hosting line and rule 5 of `## Self-Identity Discipline`) is the *intent* half; per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` it does not hold alone. **This guard closes the p1 without the companion `mika-cloud` ticket**: a cloud tenant today emits no `MIKA_DEPLOYMENT`, resolves `Unknown`, is therefore not `Local`, and the measured claim is refused from this deploy onwards. The `cloud` signal improves the *answer*; it was never needed for the *refusal*.

**Two layers, and the discrimination is of scope, not of vocabulary — that is the hard part.** The remedy the ticket prescribes contains the word "local" itself (« la MÊME stack open-source (MIT) est self-hostable en local si tu veux »), so a guard that fired on the word would forbid the true sentence it exists to make Mika say. **Layer A** is a locality predicate (`en local`, `localement`, `ta machine`, `locally`, `on your machine`, …). **Layer B** is a present-indicative assertion *carrying its own grammatical subject* (`tout tourne`, `je tourne`, `tes données restent`, `I run`, `your data stays`) — modal, conditional, interrogative and self-hosting forms are simply not in the list, which is what makes the remedy sentence structurally unmatched rather than specially excused. Both must fire, in that order, within 40 non-terminator characters (so "same sentence" is enforced by the character class rather than by a second pass).

**Layer B has two polarities and they obey opposite gap rules**, because half the measured claim was written in the negative (« tes données ne quittent **pas** ta machine »). A `pas` between assertion and locality *cancels* a positive claim ("Je tourne sur un serveur, pas sur ton téléphone" — which is the family Cloud line this ticket puts in the prompt) and *completes* a negated one. One list with one rule would have to get one of those two wrong. Contrast conjunctions (`mais`, `but`, `however`) break the predication for both polarities; a sentence ending in `?` and a narrow set of self-hosting markers suppress it. The conditional suppressors are deliberately **narrow** — the exact fragments of the prescribed remedy, not a general modal list, because `peux`/`can`/`could` would make "je peux te dire que tout tourne en local" a one-phrase bypass.

**Predicate signature takes the deployment** (`detect_false_local_hosting_claim(text, deployment)`) rather than leaving it to a caller-side `if`, so "a declared local install may say it runs locally" is a property of the pure function and carries its own test. Single retry via `intent_guard_retries`; **not** skipped by `skip_remaining_guards` (#1178) — a successful PR review grants no licence to make a false privacy claim, the same literal reason as 5c. Applies uniformly across modes; a heartbeat that asserts local hosting is exactly as false, and the compacted history hands it to the next conversational turn. **Second violation in the same turn:** the budget is spent, the EndTurn is accepted, and a distinct `guard.false_local_hosting_claim_uncorrected` WARN is emitted — the gesture 4b already makes for the milestone-close guard, so the residual population stays countable instead of merging with healthy turns. Regression scenarios: `tests/eval/doctrine_regressions/false_local_hosting_claim_caught.rs` (the measured turn, the prescribed remedy as negative control, the declared-local exemption, the retry-exhaustion boundary).

5e. **Unactioned frequency-promise guard (mika#2358):** Refuses assistant text that promises to change how often the agent sends its own **unprompted** messages — or to suspend them — while the turn called no `set_config` on `proactive_daily_budget` / `proactive_pause_until`. Position 5e, immediately after 5d, whose shape, `intent_guard_retries` budget and `guard.*` telemetry it reuses. Same family, one step further along: 5c and 5d refuse a false statement **about the world**, this one refuses a **commitment about the future** the turn did nothing to bring about.

**Founding incident, 2026-09-17, Al's cloud tenant.** Asked why he had received three technical digests when he had asked for one, Mika answered « Je vais corriger ça concrètement : plus aucun message de veille technique aujourd'hui. Et demain, un seul — pas deux, pas trois. » and called nothing. **She had nothing to call**: the only gesture reachable from a conversation was cancelling the `heartbeat` row, which expresses "none" and never "one", and which `revert_config_cancel_recurring_task` undoes at the next restart (mika#2271). mika#2358's U1 gives the promise an actor; this guard is what makes the turn reach for it. The prompt half is deliberately absent — the framing of the heartbeat turn is untouched, per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.

**Three terms, and the actor is checked FIRST.** (C) no `set_config` on either key among `all_tool_summaries`; (A) a frequency / suspension subject about unprompted messages; (B) a performative assertion. Checking (C) first is free, and it makes *"having called the tool is enough"* a property of the pure function rather than a branch of the loop — the same reasoning that put `deployment` in 5d's signature. A and B must share a sentence within `CLAIM_GAP_MAX` characters, in **either order**: French pronominalises the object it has just named, so « la fréquence de mes veilles, je **la** réduis » is the normal way to write the subject-first form, and the assertion alternation carries an optional clitic group for exactly that.

**Layer B carries its own grammatical subject** (borrowed from 5d, same reason): the vocabulary of the promise overlaps the vocabulary of the true sentence we want Mika to be able to say. « je ne peux pas régler ça moi-même » also speaks of settings and frequency and is structurally unmatched rather than specially excused — and admissions of incapacity additionally suppress at sentence scope. **That honest exit is written into the correction text as its second branch**, because a correction that only pushed towards the tool would make the model call it to be rid of the guard; the text says so in as many words ("do not call the tool merely to satisfy this message"). Layer A's `veille` requires its qualifier (`veille technique`): the bare noun is the ordinary French word for "the day before", and a guard that fired on « je vais corriger ça la veille de ton départ » would be refusing a sentence about a calendar.

**What term (C) accepts, and what it costs.** An **attempted** `set_config` satisfies it, success or failure — the `callback_terminal_action` convention. Named consequence: a promise resting on a call the tool rejected passes this guard. That belongs to the assert-grounded family (mika#1331), and widening the predicate to cover it would also re-prompt every agent that tried honestly and reported the failure.

Single retry via `intent_guard_retries`; **not** skipped by `skip_remaining_guards` (#1178) — a posted PR review changes no setting, the same literal reason as 5c/5d/6f. Applies uniformly across modes (a promise made inside a heartbeat turn is exactly as empty, and compaction carries it into the next one). Second violation in the same turn: budget spent, EndTurn accepted, `guard.unactioned_frequency_promise_uncorrected` WARN emitted — **expected regime zero**, and without it that residue would be indistinguishable from a healthy turn. Predicate units in `evidence::guards::tests::mika2358_*` (including the control that separates "reads the calls" from "reads a word": the same text passes once `set_config` is in the summaries); production-path coverage in `tests/eval/test_unactioned_frequency_promise_2358.rs` (measured turn, honest-admission negative control, factual-description negative control, retry-budget boundary). Config keys, the four-producer perimeter, operator greps and the post-deploy probe: root `CLAUDE.md` § *Une consigne de fréquence a un site d'inscription*.

5f. **Response language-drift guard (mika#2247 AC2):** Refuses assistant text measured in a language other than the one **this tenant declared** through the `language` key of `customer_config`. Position 5f, immediately after 5e, whose shape, `intent_guard_retries` budget and `guard.*` telemetry it reuses.

**Founding measurement, 2026-09-06, MikaSenara captures.** One thread flipped EN↔FR: « So — who are you… », « All good », then « Bonjour ! Je suis Mika… ». The tenant's persona (`FAMILY_SOUL`) already prescribed French **twice, once in bold**, and the drift happened anyway — which is precisely why a third sentence was refused as the remedy (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, mika#2120: nine recurrences under prompt enforcement against zero when the fact is posed by code). What was missing was a **declared** axis and a structural half, not another instruction.

**The discriminant is closed-list function words, and the reason is fail-open cheapness.** `measure_response_language` counts hits from two closed lists (`le la les de des un une et est dans que pour` / `the a an of and is in that for to`) and issues a verdict only when **three** independent thresholds clear: ≥12 word tokens, ≥3 hits for the winner, and a ≥2-hit lead. No new dependency, and function words are the part of a text a model does *not* vary — a proper noun, a code identifier or an emoji contributes to neither score. **Anything else is `Undetermined` and the guard does not fire**: « Bonjour 🌸 », « OK », « All good » must stay undecidable, because on a general-public tenant a false positive costs a needlessly re-prompted honest turn and a reply the person waits longer for. That is the expensive error here, not the missed drift.

`expected` is a parameter of the pure function rather than a caller-side `if`, for mika#2290's reason: "an undeclared tenant is never guarded" then carries its own test instead of living in one branch of the agent loop. **The third state is the common one** — every engineering agent and every un-configured tenant declares nothing, resolves `None`, and is byte-identical to the pre-mika#2247 behaviour.

Single retry via `intent_guard_retries`; **not** skipped by `skip_remaining_guards` (#1178) — a posted PR review does not license answering in the wrong language, the same literal reason as 5c/5d/5e. Applies uniformly across modes: a heartbeat that opens in the wrong language is exactly as wrong, and it is the turn that *starts* the exchange. Second violation in the same turn: budget spent, EndTurn accepted, `guard.response_language_drift_uncorrected` WARN emitted — **expected regime zero**. **What this guard does NOT do, written at the site rather than discovered:** a one-shot re-prompt does not *guarantee* AC2; it bounds it and makes the residue countable. If the post-deploy measurement shows a non-negligible residue, the remedy is an engine net (mika#2368 shape), **not a second re-prompt — and that is a ticket, not a setting**. Predicate units in `evidence::guards::tests::mika2247_*`; production-path coverage in `tests/eval/doctrine_regressions/tenant_register_held.rs` (measured drift, undeclared-tenant negative control, nominal turn). Config key, its four measured refusals of an env var, operator greps and the post-deploy probe with its three halts: root `CLAUDE.md` § *Le tenant grand-public tient son registre*.

5g. **Time-of-day greeting guard (mika#2247 AC3):** Refuses a greeting that names a part of the day the tenant is not in. Position 5g, immediately after 5f.

**It is a net, not the mechanism, and the two are tuned in opposite directions.** The measured symptom — a « belle journée » sent in the evening — had no instruction defect behind it: the prompt posed UTC and left the model **two untooled inferences** (convert to the zone, then derive a moment of day). The fix is therefore the *posed fact*: `write_time_section` now computes and renders `Local time (Asia/Singapore): 2026-09-06 20:14 / Sunday evening`, parsing both an IANA name and a fixed offset (the second because rows predating `validate_config_value` still carry `+08:00`, and reading only the first would make AC3 silently inert on exactly those tenants). This guard sits behind that fact, so its lexicon is a **closed, narrow list** where 5f's is a measurement. Two entries are deliberately permissive and say so: `bonjour` admits the afternoon (French uses it until the evening) and `bonne nuit` admits the evening (going to bed at nine is not an error).

**Fail-open on an unknown hour**, and `local_part_of_day` is taken by the pure function for 5f's reason. It is threaded from `prompt::resolve_local_part_of_day`, computed once per turn from the **same instant and the same timezone** the `## Current Time` section renders — a second parse would be free to disagree about what "evening" means, and the guard would then refuse the very greeting the prompt asked for. When no usable timezone is declared there is no local hour, so there is nothing to contradict; the prompt has already forbidden a time-stamped greeting on that path and states the ignorance rather than leaving the void that produced the symptom.

Single retry, not skipped by `skip_remaining_guards`, `guard.time_of_day_greeting_mismatch_uncorrected` for the residue — same contract as 5f above.

5h. **Testimony-grade access-proposal guard (mika#1960):** Refuses assistant text that proposes to **open** access to testimony-grade data — a Gmail grant, a full Drive, a personal journal. Position 5h, immediately after 5g, whose shape, `intent_guard_retries` budget and `guard.*` telemetry it reuses.

**What it closes.** The non-transit doctrine (mika#1798, `crates/mika-agent/docs/non-transit-data-grade.md`) is a HARD NO covering **both the doing and the proposing**, and the doc quotes the breach verbatim: *"A well-meaning 'I could help if you gave me Gmail access…' is a breach at the propose surface, **even without a tool call**."* Layers 2/3/4 of that ticket guard the *access* surface and are structurally blind to it — nothing is ever called — so until this guard the *propose* half was Layer 1 alone, which the doc itself names as the worst available state ("the fragile layer the doctrine explicitly distrusts"). Founding incident: 2026-07-18, a cloud Mika proposing Gmail / Calendar / Drive OAuth during a family-tier interaction.

**The discriminant is the DIRECTION OF THE ACCESS MOVEMENT, never the vocabulary — and that is strictly harder than 5d's.** Layer 1 *prescribes naming the doctrine when declining*, so a conforming refusal necessarily carries the forbidden subject and the word "access" in one sentence (« je ne peux pas accéder à tes emails — c'est testimony-grade »). A lexical predicate over {Gmail} × {access} would refuse the refusal, i.e. break the behaviour mika#1798 shipped by pushing the model to stop naming the doctrine. Three layers, per sentence and in **either order**: **A** testimony-grade subjects, qualified never bare (`drive.file` app-scoped is *operational by doctrine*, a bare `mail` catches postal mail); **B** the access-opening movement, positive polarity, carrying its own grammatical subject — modal, interrogative and declarative-of-incapacity forms are absent **by construction** rather than specially excused, which is what makes the prescribed refusal structurally unmatched; **C** doctrinal coverage of the sentence, kept deliberately narrow because Layer 1 asks the model to name the doctrine and a wide override would make naming it a one-phrase bypass. What removes that bypass is the **per-sentence segmentation**, not C's width: a refusal followed by a proposal in a later sentence still fires, which is the measured shape of the incident.

**No contextual parameter, and that is a decision.** 5d takes a `Deployment`, 5e a tool-summary slice, 5f a `language`, 5g a local hour — each guards a *conditional* fact. This doctrine is unconditional and the doc says so in as many words ("There is no runtime override in v1. Not a CLI flag. Not an env var. Not a DB row."), so the predicate threads nothing to the three callers of `run_loop`. Adding a conditional axis here would hand a future caller a lever to reopen the *propose* half of the HARD NO.

**The fail-safe leans towards NOT firing, and that is a cost measurement.** A false negative lets a proposal through — today's behaviour, which this cannot be worse than. A false positive re-prompts a legitimate refusal and degrades exactly what mika#1798 built. Named false negatives, stated rather than discovered: an interrogative proposal (« puis-je avoir accès à ton Gmail ? ») is suppressed with the rest of the question population, as 5d and 5e both do; and a single sentence that is both a refusal and a proposal is suppressed, because the engine would otherwise have to adjudicate which half is sincere.

**The correction text offers TWO branches**, and the second is load-bearing: refuse and name why, **or** refuse and offer an operational-grade substitute in its place. Layer 1 prescribes the latter ("offer operational-grade or non-transit assistance instead"), so a correction pushing only towards refusal would degrade the shipped behaviour — worse than the violation it catches (RK3). The text says explicitly that naming the doctrine while declining is expected, not a violation.

Single retry via `intent_guard_retries`; **not** skipped by `skip_remaining_guards` (#1178) — a posted PR review grants licence to nothing, the same literal reason as 5c/5d/5e/5f/5g. Evaluated in every mode — **but what it reads is the turn's final assistant text, and only that.** In conversation mode that text is what the user reads; in silent mode (heartbeat, callback, reminder) it is delivered to nobody, and the user channel is the `send_message` tool's input, which this guard never sees. Same blind spot for a `send_message` sent during a conversation turn, including through the #771 boundary exit that traverses no EndTurn guard. **That channel is closed since mika#2627 — see 5h-bis below**; this guard's own reach is unchanged. In silent mode 5h still keeps the proposal out of the compacted history handed to later turns. Second violation in the same turn: budget spent, EndTurn accepted, `guard.testimony_access_proposal_uncorrected` emitted — **expected regime zero**, and without it that residue would be indistinguishable from a healthy turn. **No `audit_events` row, deliberately:** the #953 family is journal-only (`guard.*` joined by `guard_correlation_id`), and inventing an SQL surface no operator query needs today would be a second population to keep in agreement. Predicate units in `evidence::guards::tests::mika1960_*` (22 tests, of which **thirteen are negative controls** — the load-bearing half); production-path coverage in `tests/eval/doctrine_regressions/testimony_access_proposal_caught.rs` (the measured turn, the retry-budget boundary, and three negative controls on the refusals Layer 1 prescribes, **seen green with the guard armed**). Behavioural half, disarmed with its reasoning at the site: `testimony_access_proposal_replayed.rs`.

**What this does NOT buy.** It does not make the *propose* surface structural in the sense of Layers 2/3/4: a post-condition guard reads outgoing text, so it **rattrape, it does not prevent**. What changed is that the half the doctrine doc distrusts stopped being prompt-only and gained a second net with its telemetry. And it closes none of the bypass classes mika#1798 names (raw HTTP, registry-ban callsite drift, testimony tools via MCP) — those are on the *access* side.

**5h-bis. The same predicate, pre-hoc on the tool input (mika#2627).** `tools::check_testimony_access_proposal` refuses an outgoing body that proposes to open testimony-grade access **before delivery and before persistence**. Not an EndTurn guard and not a re-prompt: a tool refusal, returned to the model as a `ToolOutput::error`.

**What it closes.** 5h's reach is the final assistant text; in a silent turn that text reaches nobody and the person receives what passes through `send_message`. The 5h block named that bypass in its own comment rather than claiming it closed — this is its closing, and it is the *distinct change* that comment pointed at.

**Three emitters, and the census is the deliverable (AC1).** `send_message`; plus `create_reminder` and `create_scheduled_task` when `action_type = "send_message"`, because a deferred send is still a send. **The `action_type` discriminant is load-bearing**: on `resume_agent` the message is an instruction to the agent, not a text to the person, so « rappelle-moi de vérifier si j'ai ouvert l'accès à ma messagerie » is a note to self and refusing it would be a false positive on a legitimate population. Out of population **by nature**: `delegate_task` relays its sender and emits nothing itself (covered transitively — its delegate calls `send_message`). **`run_team` is out of the tool population but NOT out of the doctrine's, and the plan's census was wrong about it** (code review of mika#2627): its completion notification wraps `run.deliverable`, which is the writer agent's LLM output (`TeamEngine::deliver`), the conversational gate's reply, or the workspace fallback on timeout — model text, sent verbatim through `message_sender` on the sync (`tools/run_team.rs`) and async (`task_engine::dispatcher`) paths. mika#2627 kept it in scan B's perimeter as a **named open channel**, not as an engine-text exemption; **mika#2633 closes it** — see § 5h-ter below, and note the guard site is *not* `TeamEngine::deliver` (which produces and does not pose) but the point of pose. **`create_scheduled_task` is wired and inert**: it is registered in no production registry (`CreateScheduledTaskTool` is constructed at one `#[cfg(test)]` site), so no `channel = "create_scheduled_task"` line can be emitted today — a correction to mika#2627's own census, named on the enum variant rather than left to be discovered.

**Refusal at CREATION, never at firing**, and that is the whole reason the two scheduling tools are guarded here: the dispatcher that fires a scheduled `send_message` has **no model to return a reason to**. It would have to delete silently (a reminder the person asked for vanishing without a word) or let it through. Named consequence: **rows created before this deploy are not covered**, a bounded and measurable population.

**Two placement properties, each load-bearing.** It runs on `cleaned` (post-`strip_internal_tags`, post-typographic-normalisation) rather than the raw argument — `cleaned` is what would actually leave, and the raw text would feed the predicate the contents of internal tags. And it precedes **both** the persistence and the mika#2134 length guard: before persistence because a proposal in `messages` is re-served by compaction, so a refusal that persisted anyway would leave the doctrine violated in the history; before the length guard because that guard's remedy is a **split**, and splitting a text that proposes Gmail access yields four texts that propose it.

**It posts NO `DeliveryVerdict`, and that is the subtlest point (RK2).** Guard 6f `unacknowledged_send_failure` reads those verdicts and refuses an EndTurn closing over an unrepaired send, counting `RefusedTooLong` as a non-delivery to repair *by splitting*. A doctrinal refusal is repairable by neither a resend nor a split, so a verdict here would re-prompt the turn for a send the doctrine refuses, suggesting the exact inverse of the right repair. The refusal therefore follows the two exits `send_message` already has for "nothing was attempted", whose reason `mika2136_les_sorties_sans_tentative_ne_posent_rien` writes down. Pinned by `mika2627_le_refus_ne_pose_aucun_verdict`.

**The refusal body offers TWO branches**, like 5h's correction and for the same reason: Layer 1 prescribes offering an operational-grade substitute, so pushing only towards a bare refusal would degrade what mika#1798 shipped (RK3). It adds what 5h has no need to say, because here the tool **failed**: *nothing was sent, the person received nothing* — the mika#2136 formula, without which the turn could announce a delivery that did not happen. **It names no workaround**: a refusal that hands over the template is a leak with one more step (mika#2520, mika#2292).

**The two guards compose and do not duplicate.** On a conversation turn the predicate can run twice — the tool body, then the final text — and the tool refusal deliberately does **not** touch `intent_guard_retries`, so 5h keeps its budget for the final text. Pinned by `mika2627_le_refus_doutil_ne_consomme_pas_le_budget_de_5h`, which is what would redden if someone routed the tool refusal through the retry set.

**Telemetry: one name, the channel in a field.** `guard.testimony_access_proposal` gains a `channel` field (`end_turn` | `send_message` | `create_reminder` | `create_scheduled_task`) — the `ready_label_outcome` motif (mika#2323), because the four populations share the same cause and call for the same operator conduct. Three fields are **absent** on the tool channels and never fabricated: `guard_correlation_id` (there is no re-prompt, hence no `guard.correction_accepted` to join), `step` and `label` (the tool site knows neither). **Dated cost:** a `jq 'select(.channel == "end_turn")'` spanning the deploy returns empty on older lines, where the field does not exist; they are not rewritten (mika#2361), so the query right on both sides is `select(.channel == null or .channel == "end_turn")`.

**No `audit_events` row**, in explicit coherence with 5h. The population stays countable in SQL without a new surface, because a `ToolOutput::error` lands in `tool_calls.output`: `SELECT tool_name, count(*) FROM tool_calls WHERE output LIKE 'REFUS (testimony-access, mika#2627)%' GROUP BY 1;`. That prefix is therefore a **wire format** with a single definition site (`tools::TESTIMONY_ACCESS_REFUSAL_PREFIX`, motif `REFUSED (cwd-guard, mika#2536)`), and that query is what justifies creating no table.

**Guards, and why none of them can be behavioural.** `mika2627_le_predicat_na_que_deux_lecteurs_de_production` freezes the predicate's production readers at two named sites — it **succeeds the by-`grep` V4 of phase 2**, which was never automated, so nothing turned red when the count changed; left in place, that V4's wording prescribed deleting the second site as a duplicate. `mika2627_tout_emetteur_sous_tools_est_garde_ou_nomme` holds a **perimeter** (not an exemption allowlist, in mika#2536's sense) compared **both ways**, so an entry outliving its site reddens. `mika2627_gated_tools_match_the_testimony_contract_constant` mirrors `TESTIMONY_GATED_TOOLS` against the actual callers in both directions — an entry with no caller is the worse direction, since it reads as coverage while covering nothing (mika#2205). All three allowlists ship **empty**; when one fires, route the site through the helper, do not add a line (mika#2201). None is replaceable by a behavioural test: a third reader or a new unguarded emitter makes **no decision wrong** the day it is written, so every assertion stays green while the coverage is silently lost (the `grooming_marker` class, mika#2158).

**What this does NOT buy.** It does not make the *propose* surface structural either: it reads an outgoing text, so it **catches before the send** rather than making the agent incapable of formulating the proposal — the house doctrine is *build the incapacity, do not promise the restraint* (mika#1991), and it is **not applicable here**, which is worth writing rather than working around: there is no capability to withdraw, the deliverable is natural-language text. It retrieves no proposal already sent and **rétro-estampille nothing**. And it leaves **RK5 open and named**: the predicate segments by sentence, so a proposal split across two `send_message` calls such that neither carries both layers escapes it — widening past the sentence would reopen the false positive segmentation exists to avoid (same class as mika#2237, which left its own degradation path open with its reason). Operator surfaces, expected regimes and the four probes: root `CLAUDE.md` § *La surface « propose » est fermée sur le canal des tours silencieux*.

**5h-ter. The team deliverable, at the point it is POSED (mika#2633).** `TeamEngine::commit_deliverable` refuses a deliverable that proposes to open testimony-grade access, **before the notification and before the persistence**. Neither an EndTurn guard nor a tool refusal: a third production reader of the predicate, with a disposition of its own.

**What it closes, and the three rectifications the code forced on the ticket.** mika#2633's AC1 names `TeamEngine::deliver` as "one site covering both paths". *(R1)* `deliver()` **produces** a text; **four** sites pose one — `deliver_phase`, the two `GateOutcome::Conversational` arms of `execute_inner`, and `apply_delegation_gate`'s `NoDelegation` arm. Guarding in `deliver()` alone covers one of four. How "reaches the person" was established: `build_run_completion_message` reads `run.deliverable` on its `Completed` arm, and sites 2/3 `return Ok(())` leaving `status == Running`, which `execute()` then promotes to `Completed` — so their text *is* wrapped by the notification. Site 4 poses `FailedNoDelegation`, whose notification arm carries a fixed engine-composed body, so its text never reaches the person. How "reaches the next run" was established: `finalize_and_shutdown` persists the field and `teams::prompt` re-serves it as `<context type="history_deliverable">` — **all four sites, site 4 included**. *(R2)* The right site is therefore the point of **pose**, and it must precede the persistence: mika#2627's own reasoning transposes word for word — *a proposal written into the history is re-served by compaction* — so guarding at `build_run_completion_message` (which both paths do call, so it would *look* like "one site, both paths") would leave the proposal in `team_runs.deliverable` and in the next run's context. *(R3)* The AC names `detect_`, and the house forbids calling it bare; but `check_testimony_access_proposal` takes a `&ToolContext` and returns a `ToolOutput`, neither of which exists in `TeamEngine` — **and a refused deliverable is repairable by neither a resend nor a split**. So the predicate's census is **extended** from two readers to three, exactly as mika#2627 extended it from one to two, and the guarantee that bounding lost is paid for by a new scan on the telemetry's **wire format**. *A census is not an allowlist: you add to it, you do not exempt in it.*

**Disposition (c), operator decision (MPC, 2026-10-02):** one re-write, then a neutral line. The re-write covers `DeliverableSource::Writer` only — a writer exists at site 1 alone, sites 2/3 carry a *decomposition reply* on a turn where `apply_delegation_gate` already practises a reinforced retry (`CONVERSATIONAL_REINFORCEMENT`), and site 4's run is already failed with a text that reaches nobody. **Fail-closed**: a timeout, an `Err`, or an **empty** result all yield the neutral line, never the original text. The empty case is a failure rather than a clean deliverable because posing it would render a notification reading "Deliverable:" followed by nothing — a silent loss where the neutral line is a stated one.

**It composes with 5h without duplicating it, and that was measured rather than deduced.** The re-write is a full agent turn, so its EndTurn traverses guard 5h, which detects the same proposal and re-prompts **on its own single-retry budget**. Two proposing responses are therefore needed before the turn returns a dirty text. Consequence worth knowing: the `team_deliverable` channel can only fire on a writer that resisted 5h, so its expected-zero regime is **doubly** protected. Pinned in both directions by `mika2633_v2_une_redaction_encore_sale_donne_la_ligne_neutre` and its control `…_une_proposition_corrigee_par_5h_ne_retient_pas_le_livrable`.

**Telemetry: the same name, a fifth channel value.** `TestimonyProposalChannel::TeamDeliverable` → `"team_deliverable"`, plus `deliverable_source` ∈ `{writer, workspace_fallback, conversational_gate, no_delegation}`, `team_run_id`, `team_name`, and `writer_agent` — present only on `writer`, an absent field rather than an empty string elsewhere. **No `agent_id`**: `team_db.agent_id()` is the team DB's hard-coded scope (`"mika"` on both production paths), not the acting agent, and the family's two other emitters carry the acting agent under that name (review finding). `workspace_fallback` (review finding) is site 1 when the writer's turn **timed out** and `deliver()` fell back to the #1128 workspace content: no writer wrote that text, so it gets the neutral line directly, never a re-write asked of the agent that just exhausted its envelope. `guard_correlation_id`, `step` and `label` are **absent** and never fabricated — there is no loop step and no mode label here (mika#2304, and `null` is not a value, mika#2331). The residue `…_uncorrected` carries the same fields.

**Guards, and why none can be behavioural.** `teams::engine::tests::mika2633_les_quatre_sites_de_pose_passent_par_le_commit` — a source scan with **two** terms (no assignment outside `commit_deliverable`, and the **cardinality** of its call sites frozen at 4, the mika#2496 motif: without it a site rewritten to pose by another route would make the pose vanish without reddening the assignment assertion). `canonical_tokens::tests::mika2633_tout_emetteur_de_la_famille_porte_un_canal` — every production emitter of the family carries a `channel`; this is what replaces the guarantee scan A gave *by accident* while the reader population was two, and **it is what measured the hole it closed**: until mika#2633 guard 5h's own residue line carried no `channel` at all, so an operator reading the family could not tell it from a future one. `…mika2633_le_bras_team_deliverable_est_inerte` — `teams/` calls the tool helper nowhere, which pins the inertia named on the enum variant. `TESTIMONY_SENDER_COVERED_UPSTREAM` gives scan B a third term (*guarded in place, **or** declared covered upstream by a file that really reads the predicate, or in the perimeter*) with a self-cleaning assertion in both directions — that is AC3's "test seen red": remove the upstream guard and `run_team.rs` counts unguarded again. All allowlists ship **empty**; when a scan fires, route the site, do not add a line (mika#2201). None is replaceable by a behavioural test: a fifth pose site, a channel-less emitter, or a second predicate reader makes **no decision wrong** the day it is written — every assertion stays green and only the coverage is silently lost (the `grooming_marker` class, mika#2158).

**Named limit, found on the way and left open with its reason.** `TeamEngine::deliver` persists its raw output to `messages` under the writer's own `agent_id` (session `team-<run_id>`) **before** the pose, so a refused deliverable can still reach that agent's compaction summary. A distinct vector with a distinct blast radius: closing it means changing `deliver()`, which this work holds unchanged on purpose. Bounded — one row, one agent, one session. Operator surfaces, the reading table, the four probes and their halts: root `CLAUDE.md` § *Le livrable d'équipe passe la garde, au point de pose*.

6. **Intent-precondition registry (#702):** Registry-driven guard that generalizes the webhook zero-tools guard (#696). `INTENT_GUARDS` is a const array of `IntentPrecondition` entries, each with a trigger function, satisfaction check, and correction message. Retry tracking uses `HashSet<&'static str>` keyed by label (one retry per entry). Current entries: (a) `webhook_ready_label_dispatch` (#846, #907, #1089) — if user message matches the `[GitHub] Issue labeled ready on` marker, requires `run_claude_pilot` attempt (dispatch via dev-pilot, or auto-groom via dev-groom). Post-#1089: the `send_message` grooming-rejection path was removed — all legitimate paths call `run_claude_pilot`; (b) `webhook_no_unauthorized_dispatch` (#910) — if user message starts with `[GitHub]` but does NOT match the ready-label marker, rejects when `run_claude_pilot` was successfully called (only successful calls — failed attempts are already blocked by the dispatch-readiness guard in `executor.rs`). Engine-level fix for recurring unauthorized dispatch from comment events (#798, #838, #910) where prompt-level source-check rules drifted under load. Post-#933: this post-hoc EndTurn guard is **defense-in-depth** — the primary prevention is the pre-hoc tool-boundary gate in `validate_dispatch_readiness()` check (0) which rejects `run_claude_pilot` before the subprocess spawns. Post-#1102: the trigger predicate now delegates to `is_unauthorized_webhook_dispatch()` from `crate::webhook_dispatch` — the same positive-allowlist predicate used by the tool-boundary guard. PR review and check-suite events (qa/ci skill territory) no longer trip the guard. Shared predicates live in `crate::webhook_dispatch` module; (c) `webhook_zero_tools` — if user message starts with `[GitHub]` and zero successful tool calls, rejects once (unchanged #696 behavior); (d) `resume_reconcile` — if user message contains resume/continue verb + milestone/project reference and no successful `check_task` or `list_tasks` call was made, rejects once; (e) `callback_terminal_action` (#870) — if user message starts with `[callback:` (Silent mode callback trigger), requires BOTH `update_task_status` AND `send_message` before EndTurn. AND-shape: both tools must be attempted (success or failure). Also has an inline mirror guard in the empty-text exit path for Silent mode, where the INTENT_GUARDS registry is not evaluated (the registry only fires in the non-empty text branch). `CALLBACK_TERMINAL_ACTION_LABEL` and `CALLBACK_TERMINAL_ACTION_CORRECTION` shared consts keep both sites in sync. **Two carve-outs, both read by both sites because they share the `callback_trigger_active` predicate:** `[callback:deferred-dispatch]` (mika#1011, own contract) and `[callback: long_running:build_mika]` (mika#2355 — a build callback owns no self_dev parent to mark terminal; the #870 audit's "only one callback flow exists" was false, `build_mika` was a second one, and imposing this contract on it let three mika-qa turns answer "Build succeeded" via `send_message` with no PR verdict on 2026-09-17). `build_callback_trigger_context` is the framing half of the same carve-out: it prescribes the self_dev terminal contract to every callback except a build callback, which is told its own (a posted `run_gh pr review`). The discriminant — label, message marker, satisfaction predicate, correction — lives in `crate::qa_build_callback` and nowhere else.
6a. **QA build-callback verdict guard (mika#2355):** Inline guard (not in `INTENT_GUARDS`), label `qa_build_callback_verdict`. Trigger is **conjunctive** — the user message starts with `[callback: long_running:build_mika]` AND `qa-review` is among the turn's injected skills (`loaded_skill_names`, a `run_loop` parameter threaded from all three call sites; in silent mode that is `callback_safe_skills()`, exactly where mika#2355 B1 made `qa-review-build-callback` reachable). Satisfied by a **successful** `run_gh` call whose input carries `"pr"` and `"review"` (`qa_build_callback::pr_review_posted_in_turn`, the same predicate as early-accept 3b). Single retry via `intent_guard_retries`, correction `QA_VERDICT_REQUIRED_CORRECTION` naming `run_gh pr review` and the `VERDICT:` line. Inline because the registry's `fn(&str) -> bool` sees the message alone, and the label does not distinguish mika-qa from mika-dev — mika-dev carries `build-mika` in its allowlist and launches builds that owe nobody a verdict; armed on the label alone this guard would trade the QA loop-breaker for a dev one (AC4b). Has an empty-text mirror in the Silent exit path like 6(e)/6b, because a bare EndTurn is the shape a turn with nothing to say takes. Coupled with the 6(e) carve-out above: the one removes the wrong contract from the build flow, the other supplies the right one. Production-path coverage: `tests/eval/test_qa_build_callback_verdict_2355.rs` drives `run_silent_agent` through both exit sites. What this guard does **not** do: post anything itself — a second EndTurn without a review is accepted (single-retry contract). What it now does on that path is **say so**: the turn raises `SilentTurnOutcome.qa_verdict_unmet`, and the engine-side `hold[review]` net reads it in the dispatcher (mika#2368 — see § *Verdict Net*, "The second reason"). The guard's one-shot budget is unchanged; the net succeeds it rather than extending it.
6b. **Callback milestone advance guard (#991):** Inline guard (not in `INTENT_GUARDS` const array) that enforces queue advancement on milestone/project-context callback turns. Triggers on `[callback:` + `[milestone-parent: <id>]` markers in the user message (the marker is injected by `run_silent_agent` after a DB lookup of the parent task type). Satisfied by EITHER Path A: `run_claude_pilot` call (advance to next child), OR Path B: `update_task_status` targeting the parent task ID with status `blocked`/`completed` (halt or finish). Inline because the satisfied predicate needs the parent_task_id from the user message. Composes with `callback_terminal_action` (entry e) — a milestone-context callback must satisfy BOTH guards. Also has an empty-text exit mirror guard. Companion `SilentTrigger::PostCallbackAdvance` fires a second advance turn if the first callback turn did not advance; auto-blocks the milestone if the second turn also fails. **Webhook companion guard (#1218, paired with #991):** Sibling inline guard for `pull_request.closed(merged:true)` webhook turns. Triggers on the `[milestone-parent: <id>]` marker prepended by `server::milestone_context_handler` when the PR-closed event correlates to a task with a `milestone`/`project` parent. Satisfaction has three valid paths: Path A (`run_claude_pilot` or `run_claude_pilot_groom` — advance), Path B (`update_task_status` on parent with `blocked`/`completed` — halt), Path C (`deploy_mika` + `send_message` — deploy-hook ack per self-dev-webhook-qa step 5.5.b). Mutually exclusive triggers with #991: the callback prefix `[callback:` and the webhook prefix `[GitHub] PR closed:` cannot both appear on a single user message. The marker parser `extract_milestone_parent_id` and the constant `MILESTONE_PARENT_MARKER` are shared with #991.
6c. **Asserted-unavailability guard (#862, #894):** Inline guard (not in `INTENT_GUARDS` const array) that detects when assistant text claims a tool is unavailable ("X is not callable", "X not callable", "I don't have access to X", "X is skill-scoped", "X skill-scoped", "cannot call X", "X is structurally not callable") while X is in the agent's turn-start enabled-tool set and no call to X was attempted in the turn. Five regex patterns with named `(?P<tool>...)` capture groups, normalized to lowercase for case-insensitive registry lookup. P2 and P4 use optional copula `(?:is )?` to catch elided-copula forms; P2 and P3 use optional adverb `(?:\w+ly )?` to catch adverb-interposed forms (e.g., "structurally", "currently"). Two-layer false-positive filter: snake-case capture constraint + enabled-set lookup. Inline rather than in the registry because it checks *assistant* text (not user input) and needs the `enabled_tool_names` snapshot + dynamic `format!` correction message. Uses `intent_guard_retries` with label `"asserted_unavailability"` for single-retry semantics. Not skipped by `skip_remaining_guards` (#1178) — a successful PR review does not grant license to fabricate tool unavailability claims. `enabled_tool_names: HashSet<String>` is a turn-start snapshot of the LLM tool array (after identity denylist + skill overrides + MCP), threaded to `run_loop` from all three call sites (conversation, silent, team). Structural counterpart to Rule 2 of `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`.
6d. **Assert-grounded guard (#1331):** Inline guard that detects affirmative state claims about referenced resources (issue/PR/task #N) without a grounding tool call (`run_gh`, `check_task`, `gh_read`) in the turn. Four regex patterns detect claim shapes (first-person verification claims, passive state assertions, handler/callback completion claims). Two-layer false-positive filter: narrow claim-verb + resource-type noun pairs, plus resource-ref extraction requirement (no ref → no fire). Satisfaction predicate checks `all_tool_summaries` for any attempt (success or failure) to a grounding tool with matching resource reference. Single retry via `intent_guard_retries` with label `"assert_grounded"`. Mirror of `asserted_unavailability` (negative → affirmative claims). Not skipped by `skip_remaining_guards` (#1178) — a successful PR review does not ground affirmative claims about unrelated resources.

**Second violation in the same turn (mika#2867):** the budget is spent, the EndTurn is accepted, and a `guard.assert_grounded_uncorrected` WARN is emitted at the point where the recidive is observed, as in the rest of the `_uncorrected` family. This is also the first emission site of mika#1381's `fabrication-detected` P0 class, and it ships **disarmed** behind `fabrication_p0_notify` / `MIKA_FABRICATION_P0_NOTIFY` (default `false`, resolved with `is_some_and` at the three `run_loop` entry points). When the gate is open, the notification is **not** sent from the branch. Guards downstream of it (6e, 6f, #1313, #864, #901, #2037) can still refuse the close, and the response that recidivated would then never go out. The notification is held in `pending_fabrication_p0` and sent through `send_with_severity(…, P0)` only once the EndTurn is accepted, just before the #953 correction telemetry, and only if the accepted text still carries an ungrounded claim. The `intent_guard_retries.contains(ASSERT_GROUNDED_LABEL)` term of the branch predicate is redundant on every reachable path, because 6d sits directly above with the same conjuncts and `continue`s. It is kept as defense in depth. Transport-level coverage, including the gate-closed control and the downstream-guard case: `tests/eval/test_fabrication_detected_p0_2867.rs`. Operator surfaces, probes and the arming condition: `docs/operator/environment-variables.md` § *La récidive d'ancrage se nomme*.
6f. **Unacknowledged send-failure guard (mika#2136):** Refuses an EndTurn that closes over a `send_message` the engine watched fail and nothing repaired. Founding incident, 2026-09-01 on Al's Telegram: a 12 000-character document refused for length, answered with « Le voici en entier 👆 »; then a split whose part 1/4 died at the transport, with the agent moving on to « Partie 2/4 » without a word. **The model had been told, both times, with the exact figures in the `tool_result`** — which is what makes this the hardest of the three same-day tickets and what rules out a prompt line as the remedy (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`: nine recurrences under prompt enforcement against zero when the fact is posed by code).

**The information already existed; what was missing was a type and a reader.** `dispatch.rs` has always pushed `{name: "send_message", success: false}` into `all_tool_summaries`, and eleven guards read that vector at close — but `grep -c "send_message" crates/mika-agent/src/evidence/guards.rs` returned **0**. So this ticket does not create a fact, it qualifies one and reads it.

**The channel is typed, never the error prose.** `ToolOutput.delivery: Option<Box<DeliveryVerdict>>` (`tools/mod.rs`), built exclusively by `ToolOutput::delivery` and modelled on `substrate_diagnostic` (mika#1783): posted by `send_message` alone, **never serialized to the LLM**, `None` everywhere else. Boxed because `ToolOutput` is the `Err` variant of a dozen `builtin_handlers` validators and an inline verdict pushes it past `result_large_err`. Reading the English error message instead would have made a sentence meant for the model into a wire format, and the three `is_error` exits do not describe the same damage anyway — a length refusal is repaired by *splitting*, a transport death by *resending the same text*.

**The verdict carries the text, and that is a decision rather than a portability detail.** It is `cleaned` (post-`strip_internal_tags`) on all six exits, because `send_message` is the sole holder of the register that was actually measured and sent. `dispatch.rs` holds no usable text for a *failed* send: its only capture site is gated on `tool_succeeded`, reads the raw `arguments["text"]`, and truncates to 200 chars. A record rebuilt from it would be wrong in three directions at once — and the error leans the dangerous way: a retry re-emitting the same content with one internal tag more or less yields two different raw values for one `cleaned`, the repair goes unrecognized, and **a line asserting a loss goes out to a user whose message did arrive**.

**The predicate is entirely structural — zero lexicon, two stages.** `evidence::guards::undelivered_sends(&[DeliveryRecord]) -> Option<UndeliveredSends>`, a pure function on the turn's full sequence: a `Failed` with no *later* record of the same text `Delivered`; a `RefusedTooLong` with no later `Delivered` at all. `Delivered` is never a term on its own — only a repair — which is what makes the happy path free **by construction** rather than by precaution. `NoChannel` / `NoSender` are in the enum and outside the predicate: both return success by design (#650, regression-guarded by mika#1090) because they are permanent session conditions, so firing on them would reopen the retry loop that ticket closed; enumerating them makes that population countable the day it gets its own ticket.

**Satisfaction is an act, never an admission read from the text — the least intuitive point.** 5c and 5d do use bilingual lexicons, but to detect a *violation*, where failing to recognize costs a false negative. Here a lexicon would recognize a *satisfaction*: failing to recognize re-prompts every agent that told the truth in unexpected words, and recognizing wrongly waves through exactly the measured case. So there is no honesty detector. Accepted cost, paid on the failure path only: one extra LLM turn per unrepaired failure **including when the agent had already explained itself well** — `test_tool_calling::test_send_message_gateway_failure_surfaces_error` is its first concrete instance and says so.

**Three mirrors, and the third is the one that matters.** Single retry via `intent_guard_retries` (`UNACKNOWLEDGED_SEND_FAILURE_LABEL`), **not** skipped by `skip_remaining_guards` (#1178) — a posted PR review makes no message arrive to anyone, the same literal reason as 5c/5d/6c/6d. Sites: (a) the non-empty-text path; (b) the silent-mode empty-text exit, which is the shape a turn whose send failed typically takes; (c) **the send-message boundary (#771) exit** — a *fourth* `run_loop` return that fires on `stop_reason == ToolUse` and traverses no post-condition at all. Without (c) the measured sequence would still close in silence in the exact mode it happened in: part 1/4 dies, part 2/4 lands, the boundary arms on that success, and the turn ends with no guard consulted. On that site the model gets its turn to *say* what failed but cannot repair by resending (the boundary stays armed) — #771's decision about how many sends a turn carries, deliberately untouched.

**On an exhausted budget the engine writes the fact itself**, diverging from 5d, which settles for an `..._uncorrected` WARN. The divergence is paid for by the damage: for 5d an exhausted budget costs one more false sentence, visible in the log; here it costs **a document that never arrived and a recipient who believes it did**. `AgentOutput.undelivered_sends` (sibling of `deadline_exceeded`, stamped on all three exits including `MaxStepsExceeded` / `DeadlineExceeded`) reaches `server::handlers`, which appends a minimal factual line **before** `sender_arc.send` — the same neighbourhood and the same reason as mika#2276's net: in conversation mode the closing text *is* the channel, and it is the one point where the engine can land a fact without going through the model. On a mute turn the line **replaces** `EMPTY_RESPONSE_FALLBACK` rather than joining it. Two registers by exhaustive `match` on `PersonaProfile` with no `_ =>` (model: mika#2290) — `FAMILY_SOUL` forbids mentioning infrastructure, so the operator wording cannot be served to a family tenant, and serving nothing would leave the void that produced the claim. **The wording is past-tense and does not exclude a later arrival, deliberately:** a `SendOutcome::Failed` is saved to `failed_sends` for a later flush, so "was not received" stays true when written where "will not arrive" would be false — and that nuance is what stops a successful flush from reading as a false positive (see halt 4).

**Guarantee, and its three named false negatives.** The engine knows *that* a send left, never *that the refused content* did. (1) **Partial coverage** — splitting into four, sending two, saying "that's everything" passes. (2) **Extinction by an unrelated send** — after a refusal, one delivered "sorry, too long, here's a summary" puts the refusal stage out while the document still never left; pinned by `faux_negatif_epingle_refus_eteint_par_un_envoi_sans_rapport`, which **reddens and names the hole** the day someone closes it rather than letting anyone believe it was covered. A length floor was declined: the threshold would be arbitrary and its error leans toward asserting a loss that did not happen. (3) **Boundary suppression (#771)** — a suppressed send never reaches `execute`, posts no verdict, and is invisible; that is #771 deciding, not a delivery failure. The transport stage has none of the first two holes: its repair is a text's equality with itself.

**What the prompt keeps, and why it cannot be structural (AC5).** One sentence extending the Grounding rule, declared as the *intent* half and never as the guarantee: the engine can state a fact and refuse a close, but **telling the user what happened, in their language and their persona's register, is not something it can write** — mika#2290 established that one fact needs two formulations and that deriving the register from a technical field is a product choice in disguise.

**Operator surfaces.** `guard.unacknowledged_send_failure` (WARN, #953 family — `stage`, `failed_index`, `failed_count`, `guard_correlation_id`, joinable to `guard.correction_accepted`): **expected regime non-zero but low**; each line is a dead send the agent was about to keep quiet, and a sustained flow is not treated by widening the guard — it says the gateway or Telegram is refusing, and *that* is what to treat. `guard.unacknowledged_send_failure_uncorrected` (WARN + an `audit_events` row `tool_name = 'unacknowledged_send_failure'`): **expected near zero**; sustained on one agent means the model is not using the re-prompt, so the half to interrogate is the guard's, not the annex's. `send_failure_annexed` (INFO) + `SELECT count(*) FROM audit_events WHERE tool_name = 'undelivered_send_annexed';` — **SOLE WRITER** is `handlers.rs`, so its *absence* under a user complaint is itself information: it says the send did not fail where you think, and points at `NoChannel` or the gateway.

**Post-deploy probe, and its two halts.** Over 7 days, read the stage distribution: if `RefusedTooLong` dominates, mika#2134's length guard is being crossed daily and it is **its** threshold to re-interrogate, not this guard. *Halt (silence):* a user reports a document announced and never received while all three greps are **empty** → do not widen the predicate; the send succeeded from the engine's point of view and the loss is downstream (gateway, Telegram, mika#2126) or in the `NoChannel` population. *Halt (false positive):* the line appears on a turn where everything arrived → disarm the annex and repair the predicate, a false line in the user channel being damage of the same order as the one repaired — **but check the `failed_sends` flush first**: a `Failed` fragment later delivered by the flush arrives *after* the line, and that is not a false positive (the line was true when written). The discriminant is a flush entry later than the turn on the same content; only in its absence is the false positive real.

7. **Persistence evaluation guard (#648):** `detect_informational_input()` checks user input for informational signals (FYI, diagnostic, correction, status update) and `detect_persistable_output()` checks assistant text for verdict-shaped patterns (root cause, confirmed, validated, lesson learned). If no persistence write tool (`store_fact`, `update_fact`, `update_core_memory`) was called and either detection matches, nudges the model once to consider calling `store_fact`. Conversation mode only. Nudge, not rejection — the model can decline. `PERSISTENCE_WRITE_TOOLS` constant defines the write-tool set.
8. **Required-suffix-line guard (#864):** Manifest-driven guard for skill-declared output contracts. Skills opt in via `[output] required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]` in `skill.toml`. `collect_required_suffix_lines()` unions lines from `Keyword` and `AlwaysOn` matched skills (not `Dependency`). The guard scans the assistant's last 3 non-empty lines (after `trim()`) for an exact match to any entry; missing match rejects EndTurn once with a corrective re-prompt naming the accept-set. Standalone retry flag `required_suffix_line_retry_done`. Position: after persistence-eval — other guards' rejections take precedence. Silent mode passes empty `required_suffix_lines` (no enforcement). Currently opted-in: `mika-arch-second-review` (GROOMED/ESCALATE) and `mika-arch-groom-ticket` (READY/ITERATE/ESCALATE). Structural counterpart for the verdict-ghosting failure mode in mika#788.
9. **Required-finding-list guard (#901):** Manifest-driven guard for skill-declared F-list emission contracts. Skills opt in via `[output] required_finding_list_prefixes = ["F1:", "F2:", ..., "F10:"]` in `skill.toml`. `collect_required_finding_list_prefixes()` unions prefixes from `Keyword` and `AlwaysOn` matched skills (not `Dependency`). The guard fires only on terminal dispositions (ITERATE/ESCALATE/Verdict: ESCALATE) — `is_terminal_disposition()` checks the last 3 non-empty lines against both the skill's `required_suffix_lines` and a `TERMINAL_DISPOSITIONS` constant. Scan range: message start up to (exclusive of) the first line matching any `required_suffix_lines` entry. At least one line in the scan range must start with a declared prefix (via `starts_with`, no regex). Missing match rejects EndTurn once. Standalone retry flag `required_finding_list_retry_done`. Position: immediately after #864 suffix-line guard. Silent mode passes empty `required_finding_list_prefixes` (no enforcement). Currently opted-in: `mika-arch-groom-ticket` and `mika-arch-second-review`. Structural counterpart for the conditional-disclosure-evasion failure class (N=8 incidents, mika#901).

### Review-anchor guard — measuring the corrective re-prompt (mika#2794)

The review-anchor guard (mika#2037, label `REVIEW_ANCHOR_GUARD_LABEL`) refuses a
non-terminal disposition — `Disposition: READY`, `Verdict: GROOMED` — whose body carries no
attestation: anchor lines quoting the reviewed brief verbatim at disjoint regions. The
predicate lives in `agent_loop::review_anchor`, whose module doc-comment carries the design;
the refusal is fail-closed and, since mika#2338, fail-visible (the disposition becomes an
`ESCALATE` of its own family, preceded by an engine finding line).

**Since mika#2794 the corrective re-prompt names the anchor that failed.** The verdict carries
`per_anchor: Vec<AnchorOutcome>` — label, excerpt, refusal — and
`agent_loop::format_anchor_diagnosis` turns it into the paragraph the model reads: the accepted
anchors by name (so the correction is local rather than a rewrite of all three), the refused
ones with the start of what they said, and — gated on `QuoteNotInBrief` — the usual cause,
stated as a likelihood: a quote absent from the brief generally comes from a document read with
a tool, not from the brief message of the turn. **Nothing in the decision reads `per_anchor`**:
the counters and the miss reason still decide alone, so the guard's refusal path is unchanged.

**Whether the model obeys is a bet, and these two journal lines are the instrument that judges
it.** The ticket's acceptance measure is *the number of withholds after a re-prompt over the
number of re-prompts*, and the two existing lines already are exactly that ratio — one per
re-prompt, one per withhold:

| line | level | cardinality |
|---|---|---|
| `event = "guard.review_anchor"` | WARN | one per corrective re-prompt (**denominator**) |
| `event = "guard.review_anchor_withheld"` | ERROR | one per withhold after the re-prompt (**numerator**) |

Both gained the same two fields: **`refused_labels`** and **`accepted_labels`**, labels joined
by `,` because a `tracing` field carries one scalar. That is what makes the measurement readable
*per anchor class* — the ticket's thesis is "it is always the first one", and only this field
confirms or refutes it in production.

Three reading rules:

- **Empty is an empty string, never an absent field.** The **absence** of either field means a
  binary older than this change (class mika#2340), never "no anchor was refused" (mika#2331).
- **No third probe.** A new line would be a second producer of the same number; the cardinality
  of the two existing ones *is* the ratio (mika#2205).
- **The instrument is not live before `make deploy`** — the journal is written by the binary
  being served, not by the checkout.

Protocol, read on `$MIKA_SPIRIT_LOG_FILE` (`/var/log/mika/server.log`):

```sh
jq -r 'select(.event == "guard.review_anchor" or .event == "guard.review_anchor_withheld")
       | [.timestamp, .event, .refused_labels, .accepted_labels, .miss_reason] | @tsv' \
  < /var/log/mika/server.log
```

**The "before" measurement is already acquired** — the offline replay of the matcher over the
real briefs and responses of three mika-arch grooming sessions, window 2026-10-18: **3 withholds
on 3 re-prompts, a ratio of 1.0**, and the refused anchor was the first one in 2 of the 3
sessions (`49ffa230`, `a7f1ccad`; `f167ff53` went 3 valid of 1 then of 0). Session `a7f1ccad`
returned the same faulty anchor three times under a re-prompt that named the class and not the
line.

The **decision** in the light of the "after" measurement — keep, reinforce, or move to
deterministic attribution (searching the refused excerpt in the turn's `tool_result` blocks, so
the sentence can say *"your A1 quotes the output of `gh_read`"* instead of supposing it) — is an
operator gesture over a deployment window. mika#2794 does not take it and opens no ticket for
it: a ticket opened on an unmeasured hypothesis is precisely what mika#2786 declined to do for
this lead.

### Deterministic Context Injection

Skills with `[context.*]` sections have their data pre-fetched by the engine before the LLM turn. `resolve_contexts()` dispatches to engine-owned handlers by `context_type`, deduplicates across skills, and returns `ContextBlock`s. `apply_context_replacements()` performs single-pass `{{key}}` template substitution on skill prompts (injection-safe — replaced content is never re-scanned). If a `required = true` context fails, the declaring skill is excluded from the turn; if `required = false`, a sentinel message replaces the placeholder. Known types: `gh_pr_diff`. Module: `skills/context.rs`.
