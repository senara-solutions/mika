# Unified Task Engine

Moved verbatim out of `crates/mika-agent/CLAUDE.md` on 2026-10-13. That file is loaded whole into a pilot's prompt as soon as the pilot reads anything under `crates/mika-agent/`; at 594 KB it no longer fit, and `crates/mika-common/tests/claude_md_size.rs` now caps every `CLAUDE.md` in the repository. Headings keep their original level.


`src/task_engine/` — single SQLite-backed scheduler. Min-heap + dedup set; 1-second tick loop; periodic DB scan (60 ticks). `TaskDispatcher` matches on `action_type`. `ensure_recurring_task()` idempotently registers heartbeat and reflection at startup. **Orphan recurring task sweep (mika#1436):** At server startup, after the filesystem walk populates `state.agents`, the engine cancels any active recurring tasks (`trigger_type='recurring'`, status in `pending|recurring_active|in_progress`) whose `agent_id` is not in the on-disk set. Cancellation preserves the audit trail (vs deletion). Companion to #1399's lazy-insert: lazy-resolved post-boot agents add their recurring tasks via `ensure_recurring_task` on demand.

**Callback/resume lifecycle:** agent creates callback task -> external process completes it -> server dispatches silent agent run with `SilentTrigger::Callback`. Loop prevention: callback turns cannot **directly** spawn new long-running tasks; the executor's `long_running_ctx == None` rejection is intercepted and re-routed through `DeferredDispatch` with `(repo, issue_number, skill)` lineage cycle detection (mika#1058). Direct spawn from callback context still hits the gate; deferred re-dispatch is the safe path.

**SilentTrigger variants:** `Heartbeat`, `Reflection`, `Callback`, `SkillRun`, `Reminder`, `PostCallbackAdvance` (#991), `DeferredDispatch` (mika#1011). Each produces correct system-prompt framing. `PostCallbackAdvance` is an engine-side structural backstop — fired by the dispatcher after a milestone/project-context callback turn completes without advancing the queue. `DeferredDispatch` is an engine-side auto-recovery for `global_dispatch_active` rejections — when `run_claude_pilot` is rejected because another dispatch is active, the engine registers a `pending` callback task with label `long_running:run_claude_pilot:deferred`. When the blocking dispatch completes, the deferred callback is promoted (status → `completed`) and dispatched on the next engine tick as a `DeferredDispatch` turn whose only required action is `run_claude_pilot` (enforced by the `deferred_dispatch_action` INTENT_GUARD). **Promotion paths (mika#1070):** (1) Inline — `dispatch_next_deferred_callback()` (`pub(crate)`) fires after `mark_task_delivered` on a non-deferred callback. mika#1124 re-added the inline anti-cascade guard at `dispatcher.rs:495`: when a `:deferred` wrapper itself completes (e.g., the silent turn no-ops), inline chain-promotion is SKIPPED — relying on the periodic backstop instead. This prevents the no-op-cascade failure mode where N wrappers drain the queue inline without ever dispatching. Real (non-deferred) callback completions still chain-promote immediately. (2) Periodic backstop — `promote_pending_deferred_if_idle()` runs every `DB_SCAN_INTERVAL_TICKS` (60 ticks), iterates over the `dispatch_class` values (`DISPATCH_CLASSES = &["implement", "groom"]` — pinned to `derive_dispatch_class` at `skills/executor.rs` via a shape test in `engine.rs`), checks `has_any_active_callback_for_class(class)` (excludes deferred wrappers via `label NOT LIKE '%:deferred'`, scopes by `COALESCE(dispatch_class, 'implement') = ?`), and promotes one wrapper per idle class per tick via `dispatch_next_deferred_callback_for_class(class)` (mika#1175). Per-class iteration prevents cross-class throughput halving when wrappers from multiple classes are co-pending. Placed BEFORE `dispatch_undelivered_callbacks` for same-tick dispatch. Fail-closed per-class on DB errors — one class's check error does not stall the other. The agent-wide siblings `has_any_active_callback()` and `dispatch_next_deferred_callback()` are kept for the inline-promotion path in `handle_task_complete` (out-of-scope for #1175, see plan § Open question 1). **Both slot predicates must exclude `:deferred` wrappers (mika#1163).** `has_any_active_callback`/`has_any_active_callback_for_class` (engine backstop) AND `has_active_callback_tasks_excluding` (per-class gate inside `validate_dispatch_readiness`) all apply `label NOT LIKE '%:deferred'`. The earlier asymmetric version caused a multi-wrapper deadlock: when two parents each held a pending wrapper, every dispatch attempt from one wrapper saw the OTHER wrapper as slot-occupied and registered yet another wrapper, with no real dispatch ever spawning. **AgentBusy recovery (mika#1070):** when `dispatch_resume_agent` returns `AgentBusy` in `handle_task_complete`, the callback keeps `completed` status (not reset to `pending`) with `next_fire_at` set to now+30s for retry delay. `dispatch_undelivered_callbacks` has a `next_fire_at` guard that skips tasks whose retry delay has not expired. γ composition: the LLM's `send_message` notification and the engine's deferred callback are independent; `validate_dispatch_readiness()` arbitrates any race. Per-agent cap of 10 pending deferred callbacks prevents flood. `cancel_task()` cascades to callback children (both immediate and deferred). **No-op wrapper detection (mika#1172 R9):** When a `:deferred` wrapper completes, the dispatcher checks `has_non_deferred_active_callback_child(parent_task_id)`. If no active child exists, emits `deferred_dispatch_noop_completion` WARN + audit event — the silent turn failed to spawn a real dispatch (mika#1124 regression signal). **Dispatch lifecycle audit events (mika#1172 W4):** Three events written to `audit_events`: `deferred_dispatch_promoted` (on each inline or periodic backstop promotion, with promoted task ID), `deferred_dispatch_registered` (on deferred callback registration at `global_dispatch_active` rejection), `deferred_dispatch_noop_completion` (on no-op wrapper detection). Promote methods (`promote_next_deferred_callback`, `promote_next_deferred_callback_for_class`) return `Option<String>` (promoted task ID) instead of `bool` to enable meaningful audit event resource_id. **(3) Force-promote (mika#1453)** — `promote_deferred_callback` agent tool (fail-closed: rejects when slot busy, no override) and `mika tasks promote-deferred <class>` CLI verb (with `--override` for cancel-then-promote). Both call `force_promote_deferred_for_class()` which shares the `has_any_active_callback_for_class()` predicate (mika#1163 parity). `find_active_callback_for_class()` identifies the blocker for the CLI override path. Three audit event types: `deferred_dispatch_force_promote_succeeded`, `deferred_dispatch_force_promote_rejected_slot_busy`, `deferred_dispatch_force_promote_override`. **What the stuck-pending reaper makes of a promoted wrapper (mika#2181).** Promotion writes `status = 'completed'`; the silent turn that consumes the wrapper only reaches `delivered` when it *returns*, minutes later under a slow model. For the reaper's predicate (`find_orphaned_pending_issue_tasks`) a deferred wrapper therefore counts as **live** when it is `pending`, **or** `completed` with a `completed_at` newer than `now - MIKA_PROMOTED_WRAPPER_LIVENESS_SECS` (default 2700s, `PROMOTED_WRAPPER_LIVENESS_DEFAULT_SECS`). The bound is load-bearing, not a rounding: on the silent-turn error path the wrapper is re-armed but never marked `delivered`, so it stays `completed` forever, and an unbounded predicate would turn that corpse into a permanent shield against repair. `delivered`, `failed` and `cancelled` are never live. `has_live_deferred_wrapper_child` (renamed from `has_pending_deferred_wrapper_child`) answers the same question with the same predicate — the two must not diverge. The `mika tasks stuck` probe takes the same two windows as the reaper, so probe and engine report on one population.

#### The direct measure that succeeds the proxy window (mika#2184)

**What mika#2181 could not reach, measured.** Its window is a proxy on the
*promotion* instant, and the residue is in the code's own doc-comment: over 30
days, **139 of 799** delivered wrappers (17 %) delivered past 2700 s, **81** of
their parents were expired `stuck_pending_no_deferred_wrapper`, and **8** of
those were expired **2820–4996 s** after promotion — out of reach of *any* value
of the constant compatible with a useful reaper. Widening it is the wrong reflex,
and the ticket's argument for that is the one worth keeping: a wrapper delayed by
a restart is a **healthy** wrapper, so a bigger number buys coverage by blinding
the reaper for longer. *A window on a proxy is a dated debt*
(`docs/solutions/best-practices/une-fenetre-bornee-sur-un-proxy-est-une-dette-datee-2026-09-05.md`).

**The chain is joined by equality, and it already existed.** The turn consuming a
promoted wrapper is a `SilentTrigger::DeferredDispatch`, and
`dispatch_resume_agent` opens its session with
`create_session_with_parent(…, task_id = Some(&task.id))` where `task.id` is **the
wrapper**. So `parent → wrappers (label = DEFERRED_DISPATCH_LABEL) → sessions
(sessions.task_id = wrapper.id) → llm_calls / tool_calls`. Stricter than mika#1652,
which has to fall back on `session_id LIKE 'team-' || r.id || '%'`. The `task_id`
column has existed since v19: no migration, no column.

**Three causes of delay, and they do NOT share a measure — this is the ticket's
own premise, corrected.** The ticket writes that the silent turn "produces those
same rows". True *when the turn runs*. A wrapper `completed` and not `delivered`
past 2700 s has three causes: **(A)** the turn runs and is slow — the AC1 case,
and the only one an activity measure covers; **(B)** `AgentBusy` — the agent lock
is held elsewhere and `dispatch_resume_agent` returns `Err` **before**
`create_session_with_parent`, so there is no session and nothing to measure;
**(C)** a service restart — nothing was running, and neither was the reaper.
This work closes **A** by measurement and **C** by an admission (see
`NotYetObservable` below); **B is not covered, and the refusal is reasoned** —
the only available signal would be "the agent has activity somewhere else", which
is true almost permanently on mika-dev and would disarm the reaper under cover of
precision. Follow-up named.

**Filtered in the application, never as a third `NOT EXISTS`.** The reflex would
be one more clause in `find_orphaned_pending_issue_tasks`. Refused, and the
refusal is written in the doc mika#2181 itself shipped: *"the shelter lives in a
SQL `NOT EXISTS`: a sheltered parent never becomes a candidate and never traverses
the application. No log, no audit, no counter […] indistinguishable from a healthy
regime."* That is the defect mika#2181 had to repair after the fact with
`find_parents_sheltered_by_promoted_wrapper`; doing it a second time in the same
function would re-dig the hole just filled. The SQL yields the candidates (proxy
window included, unchanged) and the direct measure filters afterwards, where it
can log.

**The DB returns an AGE, not a boolean.** `find_deferred_wrapper_activity_age_secs`
— `None` when no row exists, and `None` is never `0` (mika#2331). Three reasons in
order of weight: the threshold leaves the SQL and becomes a parameter of a pure
function, testable at its boundaries without a database; the log line can then
*name* the age, which mika#2277 paid dearly for not doing (two false positives read
"nominal" on first inspection because one age was reported on a disposition
crossing three); and a negative age (clock skew) clamps to `0` — "very recent",
never "very old", fail-safe towards sparing.

**Four states, and the disposition follows BOUNDEDNESS, not certainty.**
`classify_wrapper_activity(last_activity_age_secs, window_secs, engine_uptime_secs,
telemetry_armed)` is a pure function reading no global state; the disposition is an
exhaustive `match` with **no `_ =>` arm** (the `hosting_ground_truth_line` pattern,
mika#2290).

| state | disposition | why |
|---|---|---|
| `Active` | **spare** | the turn is demonstrably working |
| `NotYetObservable` | **spare** | the ignorance is **bounded**: it extinguishes itself as soon as uptime exceeds the window. Covers cause C |
| `Silent` | reap | today's behaviour, bit for bit |
| `NotRecorded` | reap + WARN | the ignorance is **permanent**: sparing here would restore the corpse-shield mika#2181 had to bound |

*What cannot extinguish itself cannot spare.* `NotYetObservable` and `NotRecorded`
are two variants rather than one `Unobservable { reason }` precisely because their
disposition differs — **a reason that decides is not a reason, it is a state**.
`telemetry_armed = store_llm_calls || store_tool_calls` is a setting that is
**read**, never inferred from an absence of rows: telling "telemetry is off" from
"the agent did nothing" is impossible by observation, which is the confusion
mika#2277 condemns.

**`MIKA_STUCK_PENDING_ACTIVITY_WINDOW_SECS`**, default **600 s** = 2× the default
turn envelope (`AGENT_TOTAL_TIMEOUT` 300 s, mika#2189), so it covers a whole turn
*and* the interval to the next call with a factor of 2 of margin. Deliberately
twice mika#1652's 300 s for team runs: the expensive error here is killing a live
turn. House three-tier parse, plus a 30-day clamp — **not** the anti-`strftime`
mechanism of `PROMOTED_WRAPPER_LIVENESS_MAX_SECS` (this threshold never enters the
SQL), but the coherence of the knob: an absurd setting would spare every parent for
ever, which is what `NotRecorded` already refuses on the other axis.

**Two event names, and that rectifies AC4's letter while holding its intent.** AC4
asks that `stuck_pending_sheltered_by_promoted_wrapper` "distinguish the two spare
causes"; the literal reading is a `cause` field on that event. **Refused:** its name
*carries* its cause, so routing an unrelated spare through it would make the name
false and split in two the population mika#2181's probe counts to measure whether
its debt is retiring. The house has an established way to keep two populations
countable apart and has used it three times — `phantom_aged_out` /
`phantom_sweep_spared` (mika#2156), `qa_deadline_verdict` / `qa_callback_verdict`
(mika#2368), `auto_pull_no_token` / `wip_rescue_no_token` (mika#2205). So
`stuck_pending_sheltered_by_promoted_wrapper` is **untouched** and
`stuck_pending_sheltered_by_activity` is its sibling, each SOLE WRITER of its own
name, pinned by `mika2184_the_two_spare_causes_have_one_writer_each`.

**One reader, held by a source scan.** `mika2184_wrapper_activity_has_a_single_reader`
refuses a second production site joining `tasks → sessions → llm_calls/tool_calls`,
**allowlist shipped empty** — when it fires, remove the second site, do not exempt
it. That is the `grooming_marker` lesson (mika#2158), which this very file has
already paid a second time with `has_pending_deferred_wrapper_child`. The needle is
a conjunction of three terms, each added by a measured false positive: it must not
accuse `find_stuck_team_runs` (whose join is a `LIKE` on `session_id`) nor Signal A
of `get_task_health_summary` (which walks the same three tables in the opposite
direction). **Deliberately not unified with mika#1652:** disjoint populations,
different joins — an abstraction drawn over two points whose joins differ is the
wrong abstraction. What is shared is the *pattern*, not the code.

**What a fresh process means for tests.** A newly constructed `TaskEngine` has zero
uptime, so `classify_wrapper_activity` answers `NotYetObservable` and the reaper
spares **everything** — correctly. Every reaper test that expects an action
therefore declares that precondition through the `observing_engine` helper, rather
than inheriting it from `Instant::now()` happening to be old enough.

**The phantom-sweep sibling (comment 1 of mika#2184) is a SEPARATE fix, and the
reason is structural.** A NULL-PID `action_type='none'` tracking row queued behind
a busy slot produces **no** `llm_calls` and **no** `tool_calls`: it has not started.
Measuring activity would spare it exactly zero times. Its discriminant is already
named, in writing, in `dispatch_liveness`'s own doc-comment — *"a tracking row still
waiting for a dispatch slot has no PID-carrying child at all — only a deferred
wrapper — so this guard cannot see it"* — and the remedy there is to consult the
deferred wrapper (`has_live_deferred_wrapper_child`, a predicate that already exists
and has nothing to do with activity). Different population, different discriminant,
different blast radius. **Precondition before opening it:** establish that the class
still recurs — the measured defect dates from 2026-09-07 and mika#2156 has since
raised the grace to 14400 s.

Operator surfaces, the five probes and their halts: root `CLAUDE.md`
§ *Optional (mesure directe de vivacité du tour différé — mika#2184)*.

#### A represented parent has nothing to repair (mika#2413)

**The failure, measured 2026-09-19.** Three tickets were un-parked at once
(#2237, #1952, #2025) and the three grooms serialised on the single `groom`
slot. The third one's parent (`e7c4e9ad`) produced four `expired` wrappers in
twenty minutes and went **`failed` at 19:19:45Z**. The loop self-healed later —
`stuck_ready_reconcile` re-dispatched and #2025 groomed — but at the cost of a
whole cycle and a transient `failed` visible on every operator surface.

**There is no TTL, and nothing expired by the clock.** Both wrapper creation
sites (`register_deferred_callback`, `rearm_deferred_callback`) build their
`NewTask` with `timeout_at: None`, and `expired` is mika#2169's *terminal
vocabulary* (`mark_deferred_wrapper_noop`: the turn happened and dispatched
nothing), not a deadline. The bound actually crossed was `MAX_STUCK_REARMS = 2`
— a **repair budget**, and the parent died in
`rearm_consumed_deferred_wrapper`'s `Unrepairable` arm, in the same second as
the third consumption. That simultaneity is the signature; no delay produces it.

**The self-sustaining loop, which is the real finding.** Contention explains only
the *first* re-arm; what condemned the parent is that the re-arm manufactured the
condition making the next turn sterile. Three individually-correct mechanisms
compose: (1) the re-arm posts a `pending` wrapper so the parent stays represented
(mika#2045), on top of the one `register_deferred_callback` already posted on the
`global_dispatch_active` refusal; (2) `execute_long_running`'s mika#1205
`already_deferred` intercept short-circuits on any `pending` wrapper **before**
`validate_dispatch_readiness`, so the next turn tests nothing — including when the
slot has since freed; (3) R9 (`has_non_deferred_active_callback_child`, whose SQL
carries `label NOT LIKE '%:deferred'`) cannot see that fresh wrapper, so the turn
that correctly re-queued is counted a no-op and re-armed again. Two confusions
compose: **R9 confuses "did nothing" with "re-queued correctly"**, and **the budget
confuses "will never dispatch" with "is waiting its turn"**.

**The fix is a predicate on the parent's state, never a list of causes.** A cause
list is an implicit wire format that drifts — a fifth cause would inherit the
wrong default — and the cause is not the right question anyway. The question the
re-arm exists to ask is *"does anything still represent this parent in the
queue?"*, and it already had an exact answer: `has_live_deferred_wrapper_child`
(mika#2181). So `rearm_deferred_callback` returns the new
`RearmOutcome::AlreadyRepresented` — **no wrapper created, no counter touched** —
when the parent carries a live wrapper. That single predicate closes both
confusions: no surplus wrapper, so the mika#1205 intercept stops short-circuiting
every turn, and a turn that finds the slot free dispatches for real.

**Two details that look incidental and are not.** (a) The **consumed wrapper is
excluded by id** (`find_live_deferred_wrapper_child(..., exclude_task_id)`): on the
`silent_turn_error` path it is still `completed` with a fresh `completed_at`, so
without the exclusion it would count itself as live and mika#2045's repair would
be dead on that path for the whole liveness window. (b) The guard is **fail-safe
towards repairing** — the opposite of the `has_non_deferred_active_callback_child`
guard immediately above it, which fails closed. An unreadable read must not be
read as "the parent is represented": that would leave a parent with nothing in the
queue and nothing to put anything back, never repaired and never expired. A wrong
"I repair" costs one point of a budget of two; a wrong "it is represented" costs
the parent.

**The guard lives in the function, and the enum makes the reapers say so.** Four
call sites reach the re-arm chain (`rearm_consumed_deferred_wrapper` ×2, the
stuck-pending reaper, the L3b stale-blocked sweep); putting the guard at the
callers would fork it four ways — the class `grooming_marker` (mika#2158) had to
close once here. The new variant is a fourth `RearmOutcome` rather than a third
use of `NotNow` for two reasons: only this path owes the consumed wrapper an
honest terminal record (a `NotNow` from `has_non_deferred_active_callback_child`
means the turn genuinely dispatched, and writing `expired` there would deny a real
dispatch), and both reapers treat any non-`Rearmed`/non-`NotNow` outcome as grounds
to **expire the parent** — so the stuck-pending reaper's two `if`s became a `match`,
and a variant added and forgotten there now fails to compile instead of destroying
tasks.

**U2 — the consumed wrapper still gets its record.** `AlreadyRepresented` writes
`mark_deferred_wrapper_noop` with a reason naming *parent déjà représenté*. Not
tidiness: on the `silent_turn_error` path the wrapper would otherwise stay
`completed`, which is exactly the status `count_promoted_undelivered_wrappers`
(L2b) reads as "promoted, never taken" — so the starvation indicator would fire
on a healthy regime, and an indicator that fires in the nominal regime is an
indicator that gets muted.

**A counter the success never clears ends up bounding something else.**
`increment_stuck_rearm_count` was the only writer, so the counter was monotone for
the row's whole life — and a groom parent **becomes** the implementation parent
(mika#1614 task reuse, `update_task_dispatch_class` flips `groom` → `implement` on
the same row). Two contentions suffered while grooming therefore condemned the
implementation before it began. `reset_stuck_rearm_count` clears it in
`execute_long_running`, **after** `spawn_long_running_exec`. **The discriminant is
"having reached a real dispatch", never "having started one"** — that is what
separates it from the reset mika#2158 had to remove, which fired on
`in_flight_self_dev` (the very action the counter counted) and made the counter
unreachable, 31 re-drives reading 1. Moving this call earlier is the regression,
and `mika2413_a_refused_dispatch_does_not_reset_the_repair_budget` is what
reddens if anyone does.

**What did NOT change, and the PR says so in answer to the 20/09 operator note:**
no dispatch policy. The `groom` cap stays 1 (`max_concurrent_for_class`, `_ => 1`),
the arch-seat serialisation is untouched, no TTL is introduced, no retry is added
(AC4 — this work *removes* undue budget spends and one surplus wrapper),
`MAX_STUCK_REARMS` stays 2, `MAX_PENDING_DEFERRED_CALLBACKS` stays 10, and no
environment variable is created.

**Operator surfaces.** `deferred_rearm_skipped_parent_represented` (INFO — fields
`parent_task_id`, `task_id` (the consumed wrapper), `live_wrapper_id`, `cause`,
`dispatch_class`). **Expected regime: non-empty under multi-un-parking, silent
outside contention** — it is the direct measure that the guard bites, and without
it a suppressed re-arm would read exactly like a re-arm that never had a reason to
happen (class mika#2205). Deliberately INFO-only, no `audit_events` row: the
population is one or two lines per parent per contention episode, and a durable row
per tick would be the churn mika#2131 bounds. `stuck_rearm_count_reset` (INFO) — a
budget cleared without a trace is the thing nobody can audit afterwards. The number
that says whether the fix took is the distribution of exhaustion causes:

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'deferred_dispatch_unrepairable_parent_failed' GROUP BY 1;
```

**Post-deploy probe, and its three halts.** At the next real un-parking of N≥3,
`deferred_rearm_skipped_parent_represented` must be non-empty and
`deferred_dispatch_unrepairable_parent_failed` empty for the `groom` class.
*Halt 1* — the second is non-empty with `cause = "noop_completion"` while the
first is empty: the guard is not biting. **Do not raise `MAX_STUCK_REARMS`**;
check the liveness window first. *Halt 2* — the first is non-empty and the parent
dies anyway: a fifth path is spending the budget. Read the `cause` and establish
the site before touching the predicate. *Halt 3* —
`deferred_dispatch_promotion_starved` (L2b) starts firing on a healthy regime: the
terminal record of U2 is not landing; post the record rather than mute the
indicator.

**Dispatcher-source arbitration (mika#1948, Porte 2).** Three dispatchers share the per-class exec slots: the **autonomous loop** (`mika_dev` — ready-label, wip-rescue mika#1852, auto-feeder mika#1863, auto-pull mika#1824), the **milestone manager** (`mika_manager`, gated behind Phase 2 promotion), and the **operator** (`operator` — interactive `/mika`, `/mika-spawn`, `mika tasks promote-deferred`). `tasks.dispatcher_source` (v51, nullable, CHECK-pinned to those three values) records which one initiated a task. NULL means a pre-v51 row and reads as `mika_dev` via `COALESCE` — the autonomous loop was the only dispatcher before the column existed. Write it with `set_task_dispatcher_source()`, a deliberate separate write rather than a `NewTask` field: `NewTask` has ~175 construction sites and all but a handful are the loop, whose correct value is exactly the NULL default.

**This is NOT the `dispatch:*` seat label (mika#2084).** The seat (`webhook_dispatch::CURRENT_DISPATCH_SEAT`, values `loop|ssc|mpc`) says which *engine* owns a TICKET and is carried on the GitHub issue; the seat gate in `validate_dispatch_readiness` refuses a ticket belonging to another engine. `dispatcher_source` says which *role inside one engine* asked for the work, and arbitrates the exec slot among them. Neither subsumes the other; they are deliberately not merged into one column.

**The seat vocabulary is written twice and guarded (mika#2092).** `KNOWN_DISPATCH_SEATS` (`webhook_dispatch.rs`) and the `dispatch:*` entries of `.github/labels.yml` are one list in two files, and they must move in the same commit. An undeclared seat is not merely undocumented: label-sync runs with `delete-other-labels: true`, so it is a label GitHub deletes from the repo and from every issue carrying it, without an `unlabeled` event — after which `classify_dispatch_seat` reads `NoSeatLabel` everywhere and the gate refuses nothing, silently. That happened on 2026-08-30 at 09:12:51Z, an hour before mika#2084 shipped, and it is why `dispatch:loop` — the label making `SeatVerdict::OwnedByCurrentSeat` reachable at all — did not exist until mika#2092. `scripts/check-dispatch-seats-declared.sh` now compares the two lists **both ways** (an orphan label resolves to `Unresolvable` and refuses the ticket, which is the mirror failure) and fails CI via the `dispatch-seats-lint` job; `scripts/test-check-dispatch-seats-declared.sh` pins its negative behaviour. `dispatch:zorglub`, the unknown-seat test fixture below, must stay undeclared.

**The seat vocabulary is written a third time, and the loop now says its own name (mika#2155).** `skills/bundled/_shared/dispatch-lib.sh` writes the literal `dispatch:loop` — `_stamp_issue_seat` adds it to the issue when dispatch-lib takes the ticket (after the #2012 groom gate, before `git fetch origin main`, from the `LABELS` snapshot dispatch-lib fetched for its own gate — a read **later** than the engine's Rust one, so a `dispatch:*` posed between the two shows up as `owned_by_other` and is logged, never overwritten), and `_release_issue_seat` removes it **before** `mika ask --task-complete` in `_deliver_callback` — the message that lets mika-dev start the next dispatch on the ticket — with a second call at the head of the EXIT trap as the crash/cancel backstop (the first successful release lowers `ISSUE_SEAT_CLAIMED`, so the second is a no-op). Releasing only at exit would let the next dispatch read a label its predecessor is about to remove, skip its own stamp (`already_owned`), and run unclaimed for its whole life. Before that, the gate was one-directional: the loop refused a ticket claimed by ssc/mpc, but a human seat had no structural way to see the loop had taken theirs. The shell copy is a literal, not `dispatch:${SEAT}`, so rule L5 of `scripts/check-canonical-tokens.sh` confronts it with `labels.yml`; Rust↔YAML stays guarded by `check-dispatch-seats-declared.sh`. Two labels, two lifetimes: `origin:loop` on the PR answers *who produced this artefact* and is permanent (mika#2026); `dispatch:loop` on the issue answers *who is writing on this branch right now* and lives exactly as long as the dispatch. The stamp never writes over another `dispatch:*` (the refusal stays the engine's job — no second classifier in shell) and never blocks a dispatch when GitHub refuses it. A `dispatch:loop` seen with no live dispatch is the residue of a run killed without its trap; the next dispatch on that ticket reads it `already_owned` and releases it on its own exit. **Named residue, not closed here:** two dispatch-lib runs genuinely live on the same ticket at once (the exec-slot lease is keyed per `(agent_id, dispatch_class)`, not per issue; the ready-label path refuses that via `live_pilot_for_issue`, the other `validate_dispatch_readiness` callers do not) would still see the first exit strip the label from under the second — incidence unmeasured; wake condition: a `dispatch_seat.already_owned` in a dispatch trace while `tasks` shows another live dispatch on the same `repo#N`. `test_stamp_issue_seat.sh` pins the site order and the four no-write populations. **mika#2649 does NOT narrow that residue, and the neighbourhood invites believing it does:** its lineage term refuses a dispatch for a ticket **other** than the event's target, so a second dispatch on the **same** ticket passes L1/L2 by construction. The two terms are orthogonal — `live_pilot_for_issue` answers *is a pilot working on this ticket?*, the lineage term answers *does this event authorize a dispatch for this ticket?* — and only the first addresses this residue.

**The exec slot is CLAIMED, not checked.** `has_active_callback_tasks_excluding` is a bare SELECT: on `None` the dispatch path proceeds, and the callback row that makes the slot observably held is written much later by the caller. In between, `validate_dispatch_readiness` performs several GitHub round-trips (issue body, open-PR, grooming markers — 10s timeout each), and four production callers enter it (`ready_label_handler`, the tool boundary, `task_engine::dispatcher`, `verdict_handler`). Two dispatchers could therefore both read "free" and both proceed — the 2026-08-30 shape, where a second writer landed on a branch SSC already had a PR open on. A slot two claimants can simultaneously believe they hold is not arbitration, it is a convention. `dispatch_slot_leases` (PRIMARY KEY `(agent_id, dispatch_class)`) makes the claim a fact: `try_acquire_dispatch_slot()` runs one `INSERT … ON CONFLICT … WHERE expired OR same-holder` inside an IMMEDIATE transaction, so the second claimant's INSERT collides rather than races. The claim is the **LAST** gate in `validate_dispatch_readiness`, on purpose — no fallible step follows it, which is why no error path needs to release a lease. Refusal is `dispatch_slot_contended`, registers a deferred wrapper like any other rejection, and writes an audit event naming the holder. The lease TTL (`DISPATCH_SLOT_LEASE_TTL_SECS`, 120s, override `MIKA_DISPATCH_SLOT_LEASE_TTL_SECS`) is what keeps fail-closed from becoming loop-breaking: it must exceed the window it guards (validation done → callback row exists, i.e. a process spawn) and stay far below a real dispatch's duration, so a dispatcher that dies mid-claim stalls its class for one TTL rather than forever.

**Operator priority.** The three dispatchers are not peers: `operator > mika_manager > mika_dev`. `promote_pending_deferred_if_idle()` consults `has_pending_operator_task_for_class()` and stands down when the operator has `pending` work in that class, so an automatic wrapper promotion cannot take the slot out from under work the operator already drafted (they cannot see the queue and would simply find their dispatch refused). This check is fail-OPEN, unlike the slot-occupancy check above it — a busy slot is information we must have, whereas priority is a preference, and fail-closing here would strand deferred wrappers and cost the loop its recovery path. `force_promote_deferred_for_class` (mika#1453) is unaffected: it is the operator's own escape hatch, not the automatic path.

**Cycle detection on the source axis.** `check_lineage_cycle` keeps its exact-tuple rule `(repo, issue, skill)` unchanged and first, then adds a narrow second rule: a `mika_manager` dispatch is refused when an ancestor in its lineage is also `mika_manager` on the SAME `(repo, issue)`, regardless of skill. That is the deadlock shape Porte 2 exists to prevent — a manager queueing behind work it started itself — and the exact-tuple rule misses it because the skill differs (`dev-pilot → callback → dev-groom`). It fires only when BOTH sides are explicitly `mika_manager`, so NULL on either side is a no-op and `mika_dev`/`operator` lineages are untouched; widening it to "any matching source" would refuse the ordinary `dev-pilot → dev-groom` chain the loop runs constantly.

**Contention reporting.** The `global_dispatch_active` rejection carries `blocking_dispatcher_source`, and `Assessment.contention_events` (`#[serde(default)]`, rendered by `Reporter` as a `### Dispatch contention` section only when non-empty) gives the manager a place to report deferrals. A pre-v51 NULL passes through as JSON `null` and renders as `unknown (pre-v51 row)` — never defaulted to `mika_dev`, so a reader can tell "we don't know" from "we observed". Phase 1 never populates this: `no_dispatch_test.rs` FORBIDDEN_TOKENS blocks any file under `milestone_manager/**` from writing `dispatcher_source = 'mika_manager'` (both SQL and Rust literal forms). Phase 2 moves that write to a dispatch-wire file outside the subtree and updates the token list in the same commit.

**Engine-level callback metadata extraction (#376):** `try_extract_callback_metadata()` parses structured fields from callback results and persists to parent task. `extract_callback_fields()` extracts `session_id`, `turns`, `cost_usd`, `duration_ms`, and `pr_url` from claude-pilot output into the `claude_pilot` metadata object. The `pr_url` field (#871 R4) is parsed from `^PR:\s+<url>` lines emitted by `skills/bundled/_shared/dispatch-lib.sh` (shared handler for dev-pilot and dev-groom, #893); the reaper (below) keys off its presence/absence.

**Cost overruns are measured on those same fields, and prevented nowhere (mika#2496 U4).** `try_report_pilot_cost_overrun()` runs immediately after `try_extract_callback_metadata()` on the `is_callback` branch of `dispatch_resume_agent` — i.e. **before** the silent agent turn, on the same parsed fields, so the measurement survives a turn that then errors or exhausts its step budget. It re-reads `claude_pilot.cost_usd` / `.turns`, and **stops nothing**. That is not timidity: the dollar half of the *« 120 turns / 40 USD »* rule has **no enforcement point upstream** — `claude-pilot`'s `_sdk_guardrail_kwargs` carries `if config.maxBudgetUsd > 0:` and ends on `pass`, and none of its application-level guardrails (`stallThreshold`, `emptyResponseThreshold`, `idleTimeoutMs`, `toolWaitCeilingMs`, `modelWaitCeilingMs`) measures dollars. So the subtype `error_max_budget_usd` is a termination vocabulary nothing can emit, and this site counts and dates a population instead of pretending to bound it. The turn half **is** enforced, at the source rather than here — `--max-turns` armed by `dispatch-lib.sh` (root `CLAUDE.md`, § *pilot turn budget*) — and the two do not substitute for one another: at the measured 0.43 USD/turn, 120 turns ≈ 51 USD, above the 40 USD threshold.
- **Threshold:** `Settings::effective_pilot_cost_alert_usd()` → `pilot_cost_alert_usd` / `MIKA_PILOT_COST_ALERT_USD`, default `DEFAULT_PILOT_COST_ALERT_USD` = 40. Prefixed, unlike its `PILOT_MAX_TURNS` sibling, because this one is read by **mika-spirit** through the config-rs `MIKA_` cascade, whose process env nothing clears; the sibling is bare-and-relayed because the pilot dispatch child clears its env (`sandboxed_pilot_env` = `env_clear()` + the positive allowlist `is_sandbox_env_allowed`) and admits neither a `MIKA_*` name nor a bare one, so it travels only through the explicit `inject_pilot_dispatch_env` relay (`PILOT_DISPATCH_ENV`), never by surviving a scrub (mika#2508).
- **Two absences that are never zeros.** An absent `cost_usd` emits nothing (mika#2331: *`null` is never `0`*) — an unmeasured dispatch is not a free one, and reporting it at zero would seed the very population this exists to size with a false member. The comparison is `partial_cmp(&threshold) != Some(Greater)` rather than `<= threshold`, and the difference is load-bearing: the two diverge on an incomparable value and they diverge the wrong way — a NaN cost is *unmeasured*, so it must fall **out** of the population, which `<=` would silently invert into a reported overrun carrying a garbage figure.
- **Coordinates are downgraded, never suppressive.** `repo`/`issue` live on the **parent**'s `reference_url`; the callback child carries a pgid and no reference (the two-row topology mika#2279 and mika#2335 had to name). An unresolvable parent yields `repo=unknown` on the line and `?` in `reasoning` — an overrun on a dispatch whose parent cannot be read is still an overrun, and dropping it would silently shrink the count.
- **Fire-and-forget**, like its four siblings: a failed audit write warns and returns. Measuring a cost must not be able to break delivery of the callback that carries it.
- **`PILOT_COST_OVERRUN_TOOL` is SOLE WRITER**, pinned by the source scan `canonical_tokens::tests::mika2496_the_cost_overrun_name_has_a_single_writer` (allowlist shipped empty, and `…_the_sole_writer_allowlist_is_empty` keeps it that way — an allowlist born empty is a place to drop the next infraction, mika#2323). The property is what makes the operator's `SELECT count(*), round(avg(CAST(after_value AS REAL)), 2) FROM audit_events WHERE tool_name = 'pilot_cost_overrun';` an **exact** count rather than a number two writers can disagree about — and that count is the explicit precondition of the follow-up ticket on `senara-solutions/claude-pilot`, which has to size the missing dollar brake. A second writer would make no decision wrong; it would make that count inexact, which is invisible to every behavioural test, hence a source scan. The scan carries its own anti-vacuity assertion (it fails if the name is written nowhere in `dispatcher.rs`), because a scan aiming at a dead name reads exactly like a clean one (mika#2103 / mika#2205). **When it fires, remove the second site — do not allowlist it.**
- **Row shape:** `tool_name = 'pilot_cost_overrun'`, `target_key = 'task:<callback-id>'`, `after_value` = the cost **and nothing else** (it is what the operator averages), `reasoning` = `repo:… issue:… turns:… threshold_usd:…` as free text. Log side: a `pilot_cost_overrun` WARN in `$MIKA_SPIRIT_LOG_FILE`, expected regime **non-empty**. Its halt: a nil count while runs are known to exceed 40 USD means `Cost:` parsing returns nothing on that population — check `extract_callback_fields` against a real `status: terminated` before concluding the fleet is under threshold. Tests: `mika2496_a_cost_above_the_threshold_is_counted`, `…_below_… is_not_counted`, `…_the_threshold_is_what_decides`, `…_an_absent_cost_is_not_a_zero_cost`, `…_an_orphan_dispatch_still_counts`.

### A recurring task caught mid-fire is re-armed, never failed (mika#2575)

`startup_recovery` step 2 marks every orphaned `in_progress` row `failed`. For a
**recurring** row that write is wrong, and the damage is not the row: it is what
the next startup makes of it.

**For a recurring task, `in_progress` is a transitory FIRING state, not a
registry state.** `claim_and_fire_task` sets it for the duration of the fire and
`update_task_rescheduled` restores `recurring_active` on return. So a recurring
row found `in_progress` at startup does not say *this scan failed*, it says *a
fire was interrupted* — and writing `failed` confuses the failure of a **fire**
with the death of a **registration**. That confusion, and nothing else, is what
arms the mika#1742 anti-zombie guard.

**The chain, read in the code rather than deduced.** (1) The seven
`ensure_recurring_task` calls run **before** `startup_recovery`
(`server/mod.rs`); (2) the guard in `create_recurring_task_if_absent` looks for
`status IN ('failed','cancelled','expired')` — `in_progress` is **invisible** to
it, so no refusal; (3) its `INSERT OR IGNORE` then collides with
`idx_tasks_unique_recurring`, whose predicate covers `in_progress`, so `n = 0`
and the call returns "already existed" — **no new row**; (4) step 2 marks the
live row `failed`; (5) `get_schedulable_tasks` selects only
`('pending','recurring_active')`, so that row never enters the heap; (6) **at the
next startup the guard now sees a `failed` inside its window and refuses
re-registration for 24 h.** No single decision is wrong — it is an *ordering*,
and link 6 turns a one-cycle outage into a day-long one.

**Measured twice.** `wip_rescue` dead ~28 h (row `ce90ad84`, fired
2026-09-28T17:00:00Z, `failed` at 17:01:21Z — one second after `mika-spirit
starting` — six restarts refused, re-armed only when the grace expired); and
mika-arch's `heartbeat` (`2b71969e`) dead ~24,5 h on 2026-09-25 by the same
chain. The second occurrence is what makes "any recurring task" a *measurement*
rather than an inference.

**Two remedies refused, each on its own cost.** *Reordering* (`startup_recovery`
before the registrations), which the ticket body proposes: alone it makes the
guard read a `failed` a few milliseconds old, so link 6 fires **within the same
startup** — it converts a one-cycle outage into an immediate 24 h one. *A
distinct status* (`interrupted_by_restart`), which the ticket comment proposes:
a new value of the `CHECK` on `tasks.status`, which SQLite cannot alter in place
— a table rebuild on `tasks`, referenced by `tasks.parent_task_id` and
`sessions.task_id`, plus a review of every query enumerating statuses (four
indexes, the dashboard, the CLI). Both remedies also share a premise worth
retiring: that the row **must** receive a terminal state. It must not.

**The fix.** `restore_recurring_after_restart` re-arms instead — `next_fire_at`
recomputed from the cron, `status = 'recurring_active'` — through the same
primitive as the nominal fire, `Database::update_task_rescheduled`. Three
properties follow, none of which needs a mechanism: the re-arm happens **in the
same startup** (step 2 precedes step 3, and `get_schedulable_tasks` selects
`recurring_active`, so the row enters the heap without waiting for the 60-tick
scan or another restart); the anti-zombie guard is **never armed** for this class
(no terminal state is written, so mika#1742 is neither modified, exempted nor
bypassed — it simply stops having a population manufactured by startup); and
`recurring_active` keeps **one writer**, since the re-arm and the nominal
rescheduling become one textual act rather than two formulations free to diverge.

**Fallback, already the house rule.** No computable cron ⇒ the row falls back to
`failed` with a named reason — exactly what `fire_task` already does in the same
case (*"cannot reschedule recurring task, marking failed"*), so the fallback
introduces no new semantics; and there the mika#1742 guard arms **legitimately**,
a recurring task whose cron will not compute having no business re-registering
every minute.

**CLI mode is the second door of the same defect, and it is closed by
abstention.** `mika chat` runs this same recovery (`cli_mode: true`) against the
database it **shares** with a possibly-live daemon. Step 2a (the A2A sweep) is
already gated `!cli_mode` with its reasoning written on site; the generic loop was
not, so launching `mika chat` while a recurring task fired killed that scan
exactly like the measured restart. The re-arm is therefore **not applied** in CLI
mode: the row is left `in_progress`, untouched. Two reasons, in order — the CLI
cannot know whether the daemon is firing right now, and it will not run the scan
it would be rescheduling anyway. If the daemon is dead, its own startup re-arms.
Strictly better than today's `failed`, and non-recurring rows keep their current
CLI treatment word for word.

**One reader for the fire-time computation.** The `metadata → timezone →
next_fire_from_cron{,_tz}` triplet existed twice (`fire_task`,
`enqueue_queued_task`) and the re-arm would have been a third.
`task_engine::cron::next_fire_for_recurring` is extracted and all **three** sites
call it; each keeps its own error disposition (`fire_task` and the re-arm mark
`failed`, `enqueue_queued_task` declines to enqueue). Pure extraction, no
behaviour change.

**No interruption counter, and the refusal is measured.** The legitimate question
is what bounds a recurring task that would kill the process on every fire, once
we stop writing `failed`. There is **no mechanism by which a scan kills the
process** (`fire_task` dispatches inside a `tokio::spawn`, where a panic is
collected as a `JoinError`; the scans shell out through `tokio::process`); the
real cause of an `in_progress` at startup is **external** (deployment SIGTERM,
operator, OOM killer) and is measured — this ticket documents restarts n°11 and
n°13 in one evening; and a low-threshold counter would fire on the **healthy**
regime (`wip_rescue` has a 5-minute cron and a fire that can last ~900 s, so on a
host restarting several times an hour — the regime this ticket documents — three
consecutive interruptions without an intervening complete fire are ordinary, and a
budget of 3 would mute the scan for 24 h on a deployment day, i.e. **reopen this
defect under another name**). A counter would also need a reset site on the
successful fire (`reset_stuck_rearm_count` pattern, mika#2413, whose written
lesson is *a counter the success never clears ends up bounding something else*),
hence a second writer on the recurring success path. What replaces it is a
**measurement**: one log event and one audit row per re-arm, single-writer, so
that if a label starts appearing several times a day the follow-up opens **with a
count** rather than an intuition.

*What is given up by saying it:* today's `failed` behaves like a circuit breaker
— it mutes the scan for 24 h after an interruption. But it trips on the **common**
case (a restart) and not on the rare one (a pathological scan, for which no
mechanism is identified): that is not a brake, it is a false classifier. Removing
it removes no real protection.

**No retroactive repair.** `ce90ad84` and its kin stay `failed`. Rewriting a
terminal state after the fact would falsify what the row said when it was written
(mika#2361 motif), and the gesture already exists: `mika tasks rearm <label>`
(mika#2446), which sets the `operator_rearm` marker the guard excludes.

**Guards.** Seven behavioural tests in `task_engine::engine::tests::mika2575_*`
(six of them **seen red** before the fix, with the one-shot negative control
staying green), five in `task_engine::cron::tests::mika2575_*`, and two source
scans. `mika2575_le_statut_recurring_active_a_un_ecrivain_unique` carries **two
terms**: T1 the literal `UPDATE … SET status = 'recurring_active'` outside
`update_task_rescheduled`, and T2 the dynamic-status write — three DB functions
take the status as a **parameter** (`db/tasks.rs` `update_task_status`,
`update_manual_task_status`, `terminal_mark_tracking_row_upstream_closed`, all of
the form `SET status = ?1`), so a future writer could set `recurring_active`
through one of them **without any `UPDATE … SET status = 'recurring_active'`
appearing in the tree**. Formulated on T1 alone, the scan would be blind to the
likelier route. T2's anti-vacuity cannot rest on the presence of the name —
`task_status::RECURRING_ACTIVE` is consumed nowhere today, so T2 is green by
**empty population**, which reads exactly like a clean tree (mika#2205); its
control is therefore on the **shape of the predicate**, by a negative fixture
**seen red** (including the multi-line form rustfmt produces past 100 columns,
which a line-anchored predicate would miss). T1 keeps its anti-vacuity by name,
and its own good-faith control **seen green** covers the doc-comment of
`db/tasks.rs::update_task_rescheduled`, which writes *"set next_fire_at and
status = 'recurring_active'"* — the loose predicate a future editor would reach
for "to be sure to catch everything" would count that prose as a second writer,
the false positive mika#2050 measured on Signal S. Both allowlists ship **empty**:
when the scan fires, remove the second site, do not exempt it (doctrine
mika#2201).

**The scan enumerates its own sources rather than reusing
`canonical_tokens::production_sources()`**, and the difference is load-bearing:
that enumerator truncates at the first `#[cfg(test)]` **wherever it is**, including
indented — and `engine.rs` carries one on a helper method far above
`startup_recovery`. Reusing it would cut the file before the fix itself, leaving
the scan green by not looking at the one file it exists to watch. The anchor on
`"\n#[cfg(test)]"` is taken from
`mika2405_the_settled_event_has_exactly_one_writer_in_production`, in this same
module for this same reason. Both helpers report the **text** of a site and never
a line number: `strip_comment_lines` *removes* the lines it discards, so any
number computed after it is off — and a wrong number in a failure message sends
the reader to the wrong place, which is why `canonical-tokens.tsv` already refuses
line numbers in as many words.

**`fired_at` is deliberately NOT stamped here, and the log field is named
accordingly.** The re-arm reschedules, it does not fire; mika#2133 reserves
`fired_at` for the last fire, which `claim_and_fire_task` will set at the next
one. Writing it here would state a false fact. The INFO line therefore carries
`last_fire` rather than `fired_at` — the guard
`mika2133_fired_at_has_a_single_literal_definition` is lexical on `fired_at` + `=`
and would count the field as a fifth writer; its message offers interpolating the
constant or refining the guard, and both are declined (the first poses the false
fact, the second widens a neighbouring ticket's predicate for a log field).

**Operator surfaces.** `recurring_restored_after_restart` (INFO — expected regime
**non-empty and low**: one line per recurring task that was in flight at shutdown,
each one a scan the restart did not kill); `recurring_restore_failed_no_cron`
(WARN — expected **empty**); `recurring_restore_skipped_cli` (DEBUG, not
instrumented: CLI population, no associated conduct). Audit: a single `tool_name`,
`recurring_restart_restore`, with the outcome in `after_value`
(`recurring_active` | `failed_no_cron`) — the `ready_label_outcome` motif
(mika#2323) rather than the two-name motif of `phantom_aged_out` /
`phantom_sweep_spared` (mika#2156), which applies when each name carries its own
cause; here both outcomes belong to the **same site** and the **same population**.
Both values are a **wire format**, frozen by test. Queries, expected regimes, the
five probes and their halts: root `CLAUDE.md` § *Une récurrente en vol au
démarrage est ré-armée, jamais échouée*.

**Out of scope, deliberately.** The mika#1742 guard and its four exemptions
(config-cancel mika#2271, unknown-trigger mika#2337, operator lift mika#2446,
`RECURRING_ZOMBIE_GRACE_HOURS`) are untouched — this removes a population startup
was manufacturing, it does not touch the predicate that read it. The CLI gating of
step 2's generic loop for **non-recurring** tasks: `mika chat` can still mark a
live `time` / `event` / `callback` task `failed`; distinct population, distinct
blast radius, and the remedy requires deciding what a CLI may sweep in a live
daemon's database — **follow-up ticket**, precondition being a measurement showing
a live task reaped by a CLI invocation. And **recurring tasks carrying a
`timeout_at`**: step 1 (`mark_tasks_expired`) precedes the sweep and writes
`expired`, which the guard also counts; the seven startup recurring tasks have
`timeout_at: None` (`task_engine/mod.rs`), so that population is empty for them,
but one created through the scheduling tool with a `timeout_at` would escape this
fix — a **named**, uncovered limit.

**Callback process liveness watchdog (#959):** `check_callback_process_liveness()` runs every 60-tick cycle and detects when a long-running subprocess (e.g., `run_claude_pilot`) has crashed without delivering its callback result. Detection: queries `in_progress` callback tasks with `process_id IS NOT NULL`, checks PID liveness via `kill(pid, 0)` + `/proc/<pid>/stat` field 22 (process start time) comparison to guard against PID reuse. On first detection of a dead process, records `first_dead_at` in task metadata; after `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS` (default 120s) elapses, re-checks task status (race guard) then marks the task `failed` with `error_reason = "subprocess_exited_without_delivery"`. Process start time is stored in callback task metadata at spawn time by `spawn_long_running_exec()`. The watchdog detects death in ~60s (one tick) + 120s grace = ~3 minutes total, vs the previous 6-hour `timeout_at` fallback. Platform: Linux only (`/proc` filesystem). The existing `timeout_at` mechanism serves as a panic-fallback for edge cases where PID tracking fails.

**Pilot silent-stall reaper (mika#2249, predicate corrected in mika#2277):** `reap_silently_stalled_pilots()` runs every 60-tick cycle **immediately after** `check_callback_process_liveness`, and the two are exact mirrors: the watchdog owns the dispatch whose process is **dead**, this one owns the dispatch whose process is **alive** and has gone silent on **every** observable surface. Nothing owned that second population before — every other reaper fires on task state — which is why `fb355061` sat `in_progress` for 2 h 18 with a live PID, an empty `result` and no terminal marker. The two select disjoint sets by construction, so the ordering costs nothing.

**Why it is in the engine and not in claude-pilot.** The founding measurement is that both mika#2246 pilots carried a *working* internal watchdog (`toolWaitCeiling=1800s modelWaitCeiling=900s` in all three logs — fields that only exist post-cpp#145) and neither fired. A watchdog starved inside the pilot's own event loop cannot fire on a pilot-side timer either, whatever that timer measures; any detector housed in claude-pilot inherits the failure it exists to see. Hardening that watchdog is `senara-solutions/claude-pilot#168`, a companion, not a substitute.

**Predicate (AC4), a conjunction whose every term has its own negative control:** (1) the population from `get_live_dispatch_callback_tasks_with_pid` (`trigger_type='callback'`, `status IN ('pending','in_progress')`, `process_id NOT NULL`) — **`pending` is where a live dispatch actually sits** (mika#2272; the callback child is created without a status, so `create_task` writes `pending`, and #525 transitions the *parent*). Scanning `in_progress` alone selected an **empty** set: 897 rows have carried a `process_id` in production and not one was ever `in_progress`, which is why mika#2261 shipped and never fired. `in_progress` is kept in the SELECT because nothing pins the state machine to this shape. The #959 watchdog deliberately keeps the narrower query — widening *that* population changes who marks a task `failed` when a process dies, which races the spawn monitor; (2) the process is **alive** via `is_same_process_alive` — a PID with no stored `process_start_time` is **declined**, because the pair that identifies a process *instance* is incomplete and a recycled PID would read as alive; (3) the **transcript** declared on the row (`PILOT_TRANSCRIPT_EXPECTED_KEY`) has not been appended to for longer than the window; (4) the **claude-pilot session log**, derived as `<pilot_log_dir>/<task-id>.log`, has not been written for longer than the window; (5) a worktree was declared (`DISPATCH_WORKTREE_FILE_KEY` → a file `dispatch-lib.sh` wrote), the declaration is readable and non-empty, the path exists, yields an mtime, and that mtime is older than `MIKA_PILOT_STALL_REAP_AGE_SECONDS`; (6) the task is **still** on one of those two live statuses on re-read, so an in-flight callback — which moves the row to `completed`/`delivered` — wins the race cleanly.

**Why three surfaces and not one (mika#2277) — the worktree-only predicate was measurably false.** On 2026-09-10 at 07:16:10Z, the first armed deploy of mika#2272 killed the only two pilots in flight, both alive and productive: their claude-pilot logs and transcripts had been written 3 min 40 s and 3 min 31 s earlier, while their worktrees had not moved since `checkout` (2758 s and 2744 s, past the window). A pilot in its reading / exploration / planning phase touches no worktree file while the SDK keeps yielding turns. **The negative control that justified the window is refuted with it**: the 24-minute inter-write gap of the healthy run `c3f9a2f9` was never the upper bound of a healthy worktree gap — the bound is the length of a reading phase, which has no measured ceiling, so *no* window is safe for a worktree-only predicate. The **transcript** does separate: `~/.mika/data/pilot-transcripts/<task-id>.jsonl` (mika#1705) timestamps every LLM turn, and on the two false positives the largest inter-turn gap was **387 s and 376 s** — a factor of 7 under the window — while being the *definition* of the class D1 targets (mika#1901: the SDK stream goes quiet) rather than a proxy for it.

**The fail-safe rule, one sentence, three terms:** *a signal that cannot be read is never a satisfied term.* Terms 3, 4 and 5 all obey it. `worktree_activity::seconds_since_last_write` and its mika#2277 sibling `seconds_since_file_write` both return `None` for an absent path, an unreadable one, no mtime, or an mtime in the future (clock skew) — plus, for the file probe, anything that is not a regular file — and every `None`, like a missing metadata key, is *not a candidate*. **That rule has a cost and AC4 keeps it from being paid in silence**: disabling `MIKA_LOG_PILOT_TRANSCRIPTS` would otherwise disarm the reaper fleet-wide with nothing said. The first time a dispatch drops out for an *unavailable* signal, the reaper emits `pilot_stall_signal_unavailable` (WARN, fields `missing_surfaces`, `label`, `age_secs`, `max_age_secs`) and stamps `metadata.pilot_stall_signal_unavailable_reported` so it says it **once per dispatch** — the same motif as mika#2040's `PILOT_TRANSCRIPT_REPORTED_KEY`. A dispatch dropped because a surface is **active** is nominal and stays silent. Disposition of that warning is emit-and-continue: a safety mechanism that halted because it could not measure would be a worse defect than the one it reports. **Since mika#2875 "the first time" means the first time past `max_age_secs`, and only for a pilot** — see the next paragraph.

**AC4's warning measured a different population than the one it names, and consumed itself before meeting it (mika#2875).** Measured 2026-10-22 on `/var/log/mika/server.log` (engine `7eb63e43`): 135 lines since 2026-10-07, all carrying `missing_surfaces = transcript,pilot_log,worktree`, and **not one** on the warning's object. Joining every `task_id` to `tasks`: **120 were `long_running:build_mika`** — not pilots, and without any of the three surfaces by construction, because the population query `get_live_dispatch_callback_tasks_with_pid` filters on `trigger_type`/`status`/`process_id` and **has no label filter**; the other **14 were pilots signalled 1 to 41 s after creation**, before a pilot could have written anything. 100 % of the lines left within 60 s of the task's creation. Since the marker is stamped on that first shot, the one case the instrument exists to see — an unavailability arising **late**, in a dispatch that had surfaces and lost them — was not merely unreported, it was **unreachable**. Class mika#2205: two populations under one name, and the name belonged to neither.

Two gates now stand at the head of `report_unavailable_liveness_signal`, **before** `missing_surfaces` is built and before any marker is written, in this order. (1) **Population** — `is_pilot_dispatch_label`, strict equality against `PILOT_DISPATCH_LABELS` (`long_running:run_claude_pilot`, `long_running:run_claude_pilot_groom`); a build is out of remit at *any* age, so this gate is first, and putting the age gate first would merely delay the 120 lines rather than remove them. Equality and not `starts_with`, mirroring `qa_build_callback::is_build_callback_label`: a prefix would admit a future `run_claude_pilot_*` tool nobody weighed. The constant is pinned against its **producer** — `skills::executor::build_callback_task`, sole writer of `label = format!("long_running:{tool_name}")` — never against a copied literal, which is the `qa_build_callback.rs:525` idiom. (2) **Age** — `dispatch_age_secs`, reading `fired_at` first (stamped at spawn by `set_task_process_id`, mika#2263, and the population query already requires `process_id IS NOT NULL`) and falling back to `created_at`; younger than `max_age_secs` is not inertia, it is a birth. **Nothing is said and nothing is stamped** — a marker laid at birth is precisely what made the late case unreachable, so the two halves are one gate, not two. An unreadable or future-dated timestamp yields `None` and therefore silence: the fail-safe rule above, applied to the age exactly as it is to the surfaces. The three new WARN fields carry what the 2026-10-22 reading had to reconstruct by hand — which population the line belongs to, and that it cleared the threshold. **What the reaper does to a pilot is unchanged**: the six-term conjunction, the kill and the transition are untouched; AC4's warning was repaired, not excepted — its N4 negative control describes a **late** unavailability now and stays green. Guards: three acceptance tests in `tests/eval/test_reaper_liveness_all_surfaces_2277.rs` (young pilot silent and unmarked; the same dispatch past the threshold saying it **exactly once**, with the marker laid at that moment; a `build_mika` producing nothing at four times the threshold, with an entry control proving it really is in the population), plus six unit tests on the two pure functions — including a drift guard that calls the label producer rather than retyping its output. Post-deploy probe: **zero** `pilot_stall_signal_unavailable` carrying a build `task_id`.

**Evaluation order is cost, not semantics.** The two single-file `stat`s run before the bounded worktree walk and an active one short-circuits it, so a live pilot no longer pays for a directory walk on every scan. The conjunction is unchanged — a surface found active ends the question either way.

**The pilot-log path is the one derived path, and that is safe here specifically.** `dispatch-lib.sh` sets `LOG_ID="$TASK_ID"` and launches `claude-pilot --log-dir "$_PILOT_LOG_DIR" --task-id "$LOG_ID"`, so the file is `<pilot_log_dir>/<callback-task-id>.log` — verified empirically against the two mika#2277 false positives. The two halves read **different** environment variables (`PILOT_LOG_DIR` in the shell, `MIKA_PILOT_LOG_DIR` in the engine; the shell's bare `PILOT_LOG_DIR` reaches `dispatch-lib.sh` only through the explicit `inject_pilot_dispatch_env` relay, because the pilot dispatch child is built by `sandboxed_pilot_env` — `env_clear()` + the positive allowlist `is_sandbox_env_allowed` — which admits no bare `PILOT_*` any more than a `MIKA_*`, mika#2508), so their **defaults** are the only place they agree without an operator — both `/var/log/claude-pilot`, pinned by `config::tests::pilot_log_dir_defaults_to_the_dispatch_lib_path`. A disagreement makes the file absent, which is `Unavailable`, which takes the dispatch out of the population: a wrong derivation can only buy inertia, never a false positive. That is why deriving is acceptable here and not for the worktree.

**The worktree path is declared, never derived.** `dispatch-lib.sh` is the sole holder (it calls `scripts/derive-worktree-path`); re-deriving it in Rust is the duplication mika-platform#58 closed, and `db.rs` states in prose why `worktree_claims` is keyed `(repo, issue_number)` with no path column. `skills/executor.rs::inject_dispatch_worktree_env` injects `MIKA_DISPATCH_WORKTREE_FILE` after the env sandbox (like `GH_TOKEN`) and stamps the path on the task once the spawn succeeded — the same trajectory and the same fire-and-forget discipline as mika#2040's transcript stamp, for the same reason: an unstamped dispatch must be invisible to the detector, never a blocked dispatch. The channel does not cross bubblewrap — bwrap wraps only the claude-pilot invocation, and the `git worktree add` runs outside it.

**Detection is unconditional; the disposition is armed (mika#2272).** `MIKA_PILOT_STALL_REAP_ENABLED` defaults to `true` since mika#2272: the reaper signals the process and transitions the task. mika#2249 landed it `false` behind a flip condition — three reviewed `pilot_silent_stall` rows with no false positive — which turned out to be **unsatisfiable rather than unmet**: it counted rows written by a detector whose population was empty, so the count could only ever stay at zero. Two live pilots went ~50 min silent on 2026-09-09 and produced no row at all.

The asymmetry mika#2249 named is real and unchanged — a false negative costs a dispatch slot, a false positive destroys hours of decision-core work — and it is now paid for by the conjunction itself: the fail-safe keeps any dispatch without a readable worktree, transcript **and** pilot log out of the population entirely, and `MIKA_PILOT_STALL_REAP_ENABLED=0` still restores observation-only without a rebuild. What mika#2272 added and #2249 never had is a **positive control on a real process** — `tests/eval/test_reaper_reaps_live_pending_pilot_2272.rs`, an authentically live silent pilot seeded through the production write path, found by the scan and killed by the disposition. **What mika#2277 added is the missing negative half**: `tests/eval/test_reaper_liveness_all_surfaces_2277.rs` carries the positive control plus **four** single-term negatives — N1 the exact incident shape (transcript and log fresh, worktree past the window), N2 transcript-fresh-only, N3 log-fresh-only, N4 signal-unavailable. Four and not one, because *an AND of three terms cannot be proven by neutralising all three at once*: a single test that freshens everything would go green on a predicate reading only one surface. The fixtures of the two earlier reaper files were updated to silence all three surfaces for the same reason — a worktree-only fixture no longer describes a stalled pilot, it describes a false positive.

The 2700 s window is unchanged and is now applied to each surface. It is very wide for the transcript (factor 7 on the measured max inter-turn gap); tightening it per surface would catch a real stall sooner, but the inter-turn distribution is measured on two runs only, so mika#2277 keeps one window and leaves the split to its own ticket.

**Open risk, carried deliberately (mika#2277):** the corrected predicate may have **no population at all**. `audit_events` holds exactly two `pilot_silent_stall` rows to date — the two false positives; no true positive has ever been reaped. The three founding cases of mika#2249 cannot be re-measured on the new signals (their transcripts were never ingested) and their pilot logs were **active** at T−5 min and T−3 min of a manual disposition taken "after CHECK mtime-worktree" — i.e. diagnosed with the predicate this ticket refutes. So the "SDK stall" class is less established than n=3 suggested. The verdict is to be taken on audit rows, not intuition; the AC4 warning is what makes the inertia legible while that accumulates.

The disposition runs **before** the audit write so `after_value` reports what happened rather than what was intended: a kill that failed and a kill never attempted must not produce the same row. `before_value` carries the status **actually read off the row**, never a constant — hard-coding `in_progress` there would make every audit row assert the very thing mika#2272 disproved.

**`failed`, not `cancelled`, behind a discriminator pre-written before the signal.** `process_kill::pre_write_cancel_reason(pid, CANCEL_REASON_PILOT_SILENT_STALL)` puts `STATUS=REAPED_PILOT_SILENT_STALL` in `/tmp/mika-cancel-reason-<pid>` first; `dispatch-lib`'s TERM trap only writes when that file is absent, so the trace names the real cause instead of `CANCELLED_BY_SIGNAL` — which `self-dev-callback` reads as an operator cancel and answers with *do NOT retry*, killing the pilot **and** the retry. The name is deliberately outside the `CANCELLED_BY_*` family and falls through that parser's branches. Re-dispatch stays with `stuck_ready_reconcile` (`auto_pull.rs`, re-drive budget from mika#2020); a retry branch in a skill prompt would be prompt-enforcement on loop substrate.

The audit row's `reasoning` carries **all three** idle ages since mika#2277 (`worktree_idle_secs`, `transcript_idle_secs`, `pilot_log_idle_secs`), not the worktree alone. That row is the only surface anyone reads this mechanism through, so it has to let the decision be replayed — and `worktree_idle_secs=2758` reported by itself is exactly what made the 2026-09-10 false positives look nominal on first inspection.

**SOLE WRITER:** `pilot_silent_stall` — this method is the only site writing that audit `tool_name` and the only site writing `error_reason = "pilot_silent_stall"` on `tasks.result`. Both strings are the operator's discriminator between silent-stall reaps, dead-PID subprocess deaths (`subprocess_exited_without_delivery`), delivered-without-PR orphans (`task_engine_reaper`) and childless parents (`task_engine_childless_reaper`); reusing either collapses two populations that need separate counts.

**Orphaned parent reaper (#871):** `reap_orphaned_parent_tasks()` runs every 60-tick cycle (same cadence as other periodic scans) and detects parent self_dev tasks left `in_progress` after their callback subtask delivers without producing a PR. Detection query (`find_orphaned_parent_tasks`): parent `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'`; child `trigger_type='callback'`, `action_type='resume_agent'`, `status='delivered'`; child `updated_at` older than `REAPER_GRACE_SECONDS` (600s); parent metadata has no `$.claude_pilot.pr_url`; `NOT EXISTS` active sibling guard (defers when #870's retry loop launched a new callback child). On match: transitions parent to `failed` via guarded `update_task_failed` (terminal-state check prevents TOCTOU race), emits `audit_events` row with `tool_name='task_engine_reaper'`. Pre-existing leaks (age > 24h) get a distinct log line for post-deploy backfill visibility.

**Parent auto-completer (mika#1162):** success-side coupled pair of the reaper. Covers the gap where callback delivers with `pr_url` (success indicator) but the silent agent turn fails to call `update_task_status` (timeout, max-steps continuation that drops the call, transport error). Two layers: (1) inline `try_complete_parent_on_callback_success` in `dispatcher.rs` runs alongside `try_extract_callback_metadata` (#376) and `try_promote_parent_on_retry_success` (#958) on every callback delivery — fires immediately so the dispatch slot frees fast. (2) periodic `complete_parent_tasks_on_callback_success` in `engine.rs` runs every 60-tick cycle as a backstop for crash-recovery cases (server died between callback delivery and the inline call) and pre-deploy wedges. Both paths use the same `find_completable_parent_tasks_on_pr_url` DB query — identical join shape and guards to the reaper but inverted on the `pr_url` predicate (`IS NOT NULL` instead of `IS NULL`), so the two queries never select the same row. Both paths transition `in_progress → completed` via `update_task_completed` (which guards `status IN ('pending', 'in_progress')` — concurrent operator cancels or agent updates lose the race cleanly) and emit `audit_events` row with `tool_name='task_engine_parent_completer'` and a reason string starting with `parent_completed_from_callback`. Inline-path reason carries the literal `parent_completed_from_callback (pr_url: <url>)`; periodic-path reason carries `parent_completed_from_callback_backstop (pr_url: <url>)` for source distinction. SOLE WRITER: `task_engine_parent_completer` audit events come from these two sites only — coupled pair with `reap_orphaned_parent_tasks` (failure-path SOLE WRITER of `task_engine_reaper`). Together the two backstops cover every (callback outcome × parent state) combination.

**Dispatch-refusal resolver (mika#2158):** the refusal-side third sibling of the two above. `try_resolve_parent_on_dispatch_refusal` runs inline on every callback delivery and transitions the parent `in_progress → completed` when the callback body is a `dispatch-lib` auto-skip (`{"status":"auto_skipped","reason":"…"}` — canonically `already_groomed`, mika#2012 exit semantics via `_deliver_callback` + `exit 0`).

**Why `completed`, not `failed`:** the refusal is a *correct outcome* — the ticket genuinely is groomed. Marking it failed would say something went wrong and would pollute the count of real breakages that `task_engine_reaper` exists to hold.

**Why it matters beyond a tidy row.** Before this, the refusal reached the engine and produced nothing: the tracking row `ready_label_handler` pre-created stayed `in_progress` for ~60 min until the mika#1712 phantom sweep failed it. Measured 2026-09-03: 31 such rows on mika#1772, 8 on #2127, 7 on #2108, every one `phantom_aged_out`. For those ~60 min the ticket read `in_flight`, which the auto-pull Phase 2 reconciler treated as *progress* and used to zero the re-drive budget — so the guard meant to bound the loop was erased by the loop, every tick. The mika#2158 companion change in `auto_pull::classify_stuck_ready` (`in_flight` now skips **without** resetting) is the other half; neither closes the livelock alone.

**Scope:** all `auto_skipped` reasons, not just `already_groomed`. The phantom mechanism is identical for `issue_closed` (mika#988) and for any future refusal; the reason is carried verbatim into `tasks.result` and the audit row, so the populations stay separable. Filtering on the one measured reason would leave the same defect open under another name.

**SOLE WRITER:** `dispatch_refusal_resolver` audit rows come from this site only. That is load-bearing for attribution (mika#2158 M6a): a `phantom_aged_out` row used to be compatible with two incompatible stories — the pilot never ran, or `dispatch-lib` refused and the refusal never landed on the engine. Now the second leaves a named trace, so its **absence** under a phantom is itself evidence for the first.

**Childless-parent reaper (mika#1687):** `reap_childless_stuck_parent_tasks()` runs every 60-tick cycle after the auto-completer (so delivered-child success/failure cases resolve first). The deterministic backstop for **silent pilot death** — a parent that reaches `in_progress` but never records a callback child falls through all three sibling mechanisms above: the orphan reaper and auto-completer both INNER-JOIN a delivered callback child, and the watchdog keys off the callback child's PID. Detection query (`find_childless_stuck_parent_tasks`): parent `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'`, `type='issue'`, `updated_at` older than `MIKA_CHILDLESS_PARENT_REAPER_GRACE_SECS` (default 1800s / 30 min — far larger than the orphan reaper's 600s because a legitimately-dispatching parent is childless only for the sub-second window between its `pending → in_progress` transition and the callback-child row commit), and `NOT EXISTS (SELECT 1 FROM tasks child WHERE child.parent_task_id = parent.id)` — the **exact complement** of the sibling reapers' INNER JOIN, so the three selection sets are disjoint by construction. On match: transitions parent to `failed` via guarded `update_task_failed` (terminal-state check prevents TOCTOU race), emits an `audit_events` row plus an INFO `task_engine_childless_reaper.reaped` log (with `age_minutes`); reuses `get_reaper_child_snapshot` for a `.evaluated` log confirming `children_count == 0` at decision time. Its job is **fail-with-telemetry, not re-drive** — freeing the dispatch slot + emitting a greppable signal; re-driving the still-open ticket is mika#1824's job at the auto-pull/label layer. Scoped to `type='issue'` in v1 — milestone/project childless-stuck detection is a deferred follow-up (they carry their own advancement backstops #991/#1218). **SOLE WRITER:** distinct `error_reason = "stuck_in_progress_no_callback_child"` (on `tasks.result`) and distinct `tool_name = 'task_engine_childless_reaper'` (on `audit_events`) — neither string is written anywhere else; reusing either breaks the operator/monitor discriminator that counts silent-pilot deaths separately from delivered-without-PR orphans (`task_engine_reaper`) and dead-PID subprocess deaths (`subprocess_exited_without_delivery`).

**Supersede-on-new-dispatch (mika#1934 AC2, mika#2263, mika#2335):** `tracking_cleanup::supersede_prior_tracking_rows` cancels the stale tracking rows a fresh dispatch for a `reference_url` replaces, and — since mika#2263 — disposes of the pilot still running under them. Fail-open end to end: superseding is a courtesy cleanup, never a precondition for dispatch.

**A dispatch is TWO rows, and forgetting it cost mika#2263 its entire effect.** The **parent** tracking row is `manual` / `action_type='none'`, carries the issue URL, and **never** carries a `process_id`; the **callback child** is `callback` / `resume_agent`, carries the pgid, and **never** carries a `reference_url`. mika#2263's kill resolved its population with a query keyed on both at once (`process_id IS NOT NULL AND reference_url IN (…)`) — a conjunction **empty on the topology production writes**, so it could never kill a dispatch pilot, in that incident or any other. Measured 2026-09-15: the log of the surviving pilot `590a06c0` contains zero occurrences of `SIGTERM`, `CANCELLED_BY`, `superseded` or `Killed`, and two pilots wrote to the `feat-2334` worktree for **28 min 54 s**. The general rule worth keeping: *a resolver written a second time is a resolver that can disagree with the first* — the traversal that works already existed and was tested (`find_dispatch_children_with_pid`, mika#2156, whose own doc says "the missing link is read here, not added"). The dead query was **deleted**, not left in place; one that can return nothing is read as a guarantee by whoever finds it next.

**Order and population.** The candidate parents are computed **once**, before the kill, and reused for the row cancellation — the two halves used to run independent lookups that could diverge. The processes die **before** the row bookkeeping, deliberately: the fresh dispatch is about to claim the same worktree. Two filters bound the population, and both matter now that the kill lands: a child whose status is terminal has no pilot left to kill (its pgid is stale), and a child with **no readable `process_start_time` is not signalled** — `kill_process_gracefully` would fall back to a bare `/proc/<pid>` check, which cannot tell the pilot from a recycled PID, and mis-signalling a process *group* is unbounded damage. That child survives its supersession; the cost is named rather than hidden, counted under mika#2156's `unusable_child_count` vocabulary. Inertia, never a blind kill.

**`fired_at` on the parent (mika#2335 F2a).** `Database::mark_parent_dispatched` is the SOLE WRITER of the parent-side dispatch transition, and the **three** production dispatch paths go through it: `skills::executor::execute_long_running` (#525, the original), `server::ready_label_handler` and `server::verdict_handler` — the last two exist because the first was copied, and say so in their own comments. mika#2263 stamped `fired_at` in `set_task_process_id`, which is correct and is the **child's** writer; the parent — the row `mika tasks`, the dashboard and the health probes actually read — went through `update_manual_task_status`, which writes `status`, `updated_at` and `completed_at` and nothing else. So a live dispatch's parent read "never fired" for the pilot's whole life, and on 2026-09-15 an operator read exactly that and cancelled a pilot 38 minutes into its work. A **named writer** rather than a `CASE` added to `update_manual_task_status`, because that method also serves `rewind.rs`, which restores a prior status and must stamp nothing. An existing `fired_at` is never overwritten — the reapers measure a dispatch's age from it. **Blast radius, censused:** the only predicate reader of `fired_at` is the `long_running` probe in `get_task_health_summary`, which filters `trigger_type != 'manual'` and therefore never sees a parent; every other reader is a display surface.

**Guard:** `db::tests::mika2335_no_production_dispatch_transitions_a_parent_without_stamping` — a source scan refusing a fourth `update_manual_task_status(…, "in_progress")` in production code, with an **empty allowlist** (the three sites migrated; none is exempt). A behavioural test cannot catch this class: the regression makes no covered decision wrong, it leaves a *new* path mute while every existing assertion stays green. Its scope is lexical on the `"in_progress"` literal — `rewind.rs` and `tools/update_task_status.rs` pass the status by variable and escape it structurally, which is correct (neither dispatches), but a future dispatch path building its status in a variable would pass underneath. Disposition for a fourth site is **halt-and-surface**, not an allowlist entry: whether it dispatches a parent is a question the guard cannot answer for you.

**Operator surface (mika#2335 F2b/F2c).** `mika tasks list|get` now report **measured** pilot liveness (`is_same_process_alive` + the claude-pilot log mtime) instead of inferring "executing" from the presence of a PID column, and on a **parent** row they traverse to the dispatch child — before this, the row an operator consults showed nothing at all. `mika tasks cancel` warns, names the PID and the log age, and asks for confirmation when a pilot is alive (`--yes` / `-y` to skip; refused outright in a non-TTY without it). It warns and asks rather than refusing: killing a live pilot is sometimes exactly the intent. This is the tool-side engraving of the operator lesson the incident produced — *never cancel without checking the PID and the log mtime.*

**Live-pilot predicate (mika#2279):** `crates/mika-agent/src/live_pilot.rs` — **sole reader** of "is a pilot alive for this ticket?". Two surfaces ask it, the ready-label handler's gate 2c and the `auto_pull` Phase 2 filter 4b, and one module answers.

**The topology is the whole difficulty, and it is the same two lines mika#2335 had to name.** The URL lives on the parent, the pgid on the child, nothing carries both — so every predicate reading a single row is blind to half the state. `has_active_self_dev_task_for_issue` conjoins `reference_url IN (…)` — the closed variant set since mika#2638, a prefix `LIKE` before it — with `status IN ('pending','in_progress')` on one row, and that conjunction goes **false** the instant a supersession cancels the parent while its pilot keeps working. `find_dispatch_children_for_issue_url` traverses child → parent on `parent_task_id` with **no predicate on the parent's status**: that absence *is* the fix, a cancelled parent being exactly the state where the question needs asking.

**The loop it closes, measured on #2276 (2026-09-10).** A second `labeled ready` landed 31 s after the dispatch. Supersede cancelled the parent — and since mika#2335 that also **kills** the running pilot — then the readiness check failed and a duplicate deferred dispatch was registered; `auto_pull` then read the cancelled parent as "nothing in flight" and re-drove the label. One full turn every ~20 min, every GitHub event by `mika-platform-bot`, no human in it. The defect is self-sustaining, and each turn spends one point of the mika#2020 re-drive budget — three turns abandon a healthy ticket to `operator-review`.

**Where the guard belongs.** Not in `supersede_prior_tracking_rows`: disposing of the pilot a **new** dispatch replaces is the supersession's explicit contract, and guarding it there would undo mika#2335. A replayed `labeled` event is not a new dispatch, so the guard is on the trigger. Gate 2c sits between the repo allowlist (2b) and the token (3) — before any `gh issue view`, since the predicate reads the issue's URL and never its body, and before 6b/7, which is the property the three neighbouring gates already state: zero task created, zero process signalled, zero dispatch deferred. It refuses in `Handled`, never `Passthrough`, for the reason written at each of its neighbours.

**Fail-safe, and the asymmetry that decides it.** `Unreadable` is not `Alive`: an unreadable `process_start_time`, a `process_id` outside `u32`, a DB error — none can *prove* a pilot is alive, so none blocks, and the pre-fix behaviour is resumed. That population is the one mika#2335 counts under `unusable_child_count` and is the only shape in which #2279 can still occur; it is named rather than hidden. A false `Alive` freezes **one** ticket and the freeze is bounded (the PID watchdog #959, the silent-stall reaper #2249/#2277 and the phantom sweep #1712 all make the row terminal, after which the predicate goes false on its own); a false `None` replays the incident in a loop. `LivePilotVerdict::is_alive()` exists so a caller cannot write `!matches!(v, None)` and invert that.

**Cost, bounded by ordering rather than by a cache.** Phase 2 resolves the fact **only when `in_flight` is false**, and `classify_stuck_ready` keeps its `live_pilot` branch *after* the `in_flight` one — pinned by `mika2279_the_nominal_in_flight_case_is_still_named_in_flight`, because a branch that drifted above would both charge the nominal case a `/proc` probe and merge two populations that must stay countable apart. The verdict is `Skip`, never `SkipAndResetBudget`: waiting for a pilot is right, calling the wait a success is what made the mika#2020 guard unreachable (mika#2158).

**Phase 2 dispatches in-process; the churn is age and redundancy, never the sole trigger (mika#2470).** The stuck-ready net exists to survive the loss of the inbound webhook channel — and until mika#2470 its only action was a remove→add churn of `ready` whose `labeled` event had to *come back through that channel* to dispatch anything. Measured 2026-09-21 (eno1 unplugged, 0 webhooks for hours): three `stuck_ready_reconciled` lines, `count(tasks) = 0`. The rescue loop now calls `server::ready_label_handler::try_handle_ready_label_dispatch_with_fetcher` **directly, before the churn**, handing it the body and labels the tick already read as the injected fetcher (zero `gh issue view` per rescue; the handler decides on the same snapshot the Phase 2 filters used). Its fifteen gates apply unchanged, by call rather than by copy — `ready_label_handler.rs` carries a zero diff. Dispatch-before-churn is a **timing margin on a live channel, not an ordering invariant**: step 9i is a `tokio::spawn`, so the pgid is written by the detached task *after* the handler returns `Dispatched`, and gate 2c refuses the churn's `labeled` as `pilot_in_flight` only once that write has landed (milliseconds, against two `gh` round trips plus the GitHub → gateway → agent hop; a `labeled` that beats it lands in the step 7 → 9i window where 6b would kill the newborn, mika#2335 — probe S2 measures the margin: two `gate=dispatched` within a minute is the halt). On a non-`Dispatched` outcome that left a `pending` parent (9d slot taken → deferred wrapper; 9a/9b tool missing), the churn's `labeled` passes 2c and 6b, collides at step 7 on the active-URL index and returns `Passthrough` — one LLM turn spent on a ticket the engine already holds, no second pilot, no kill. The churn stays because it resets the label age (the mika#1824 D3 throttle; mika#2020 budget unchanged) — **except when the engine already holds the ticket** (third-pass mika-arch, Option A): `churn_is_moot(action, engine_holds)` = not `Dispatched` **and** `has_active_self_dev_task_for_issue(url)`; then the audit line carries `churn=skipped_in_flight`, the ledger records `in_flight_self_dev`, no re-drive budget point is spent, and the loop `continue`s — the age reset would throttle nothing (filter 4a excludes the ticket next tick) and the redundant trigger could only collide. A probe error churns as before (fail-open to the older shape). `rescued` counts churned rescues only; the audit line's `churn=` field says which. The per-tick cap `MAX_STUCK_RESCUE_PER_TICK` bounds **attempts** (counted before the direct call), not churn successes: a `gh_remove_label` failure `continue`s without a rescue, and every attempt runs the handler's readiness chain with real side effects. `DirectDispatchCtx { skills, global_home_dir }` descends from `TaskDispatcher` and is **not** an `Option` — an absent context is a compile error, not a silent return to label-and-wait. **SOLE WRITER** of `tool_name = 'stuck_ready_direct_dispatch'` (`target_key = issue:<n>`, `after_value ∈ {dispatched, handled, passthrough}`, `reasoning = issue=<n> action=<a> task_id=<id|none>`, under the tick's `trace_id`): `dispatched` is the primary success, the `self_dev` parent its topological consequence; on `handled`/`passthrough` join `ready_label_outcome` on `trace_id` to read the gate. `task_id` is known only for `Dispatched`; a `handled` line may still have left a `pending` parent (slot taken, tool absent) — find it by `tasks.reference_url`. Order pinned by `mika2470_direct_dispatch_precedes_the_label_churn` (source scan).

**Filter B is not redundant with gate A.** "Terminal parent + live pilot" has other legitimate producers, the clearest of which is written in the code itself: when the supersession's kill does **not** land (EPERM, survival past SIGKILL), `tracking_cleanup` deliberately leaves the row and its pgid intact so a reaper can still reach the process. Add `mika tasks cancel --yes` and the grooming branch of `tools/create_task.rs`. **Out of scope, deliberately:** the LLM path into the supersede (`tools/create_task.rs`), whose legitimacy is not settled by this predicate.

**Operator surfaces.** `ready_label_pilot_in_flight` (INFO + an `audit_events` row of the same name, keyed on the issue reference — no `task_id` exists at that point by construction, which is the property the gate is defined by), and the `live_pilot_orphaned_parent` exclusion filter on the `auto_pull` side. Queries, expected regimes and the 48 h post-deploy probe: root `CLAUDE.md` § *Un `labeled ready` répété sur un pilote vif est un NO-OP*.

**Launcher-health predicate (mika#2634 phase B):** `crates/mika-agent/src/pilot_launcher_health.rs` — **sole reader** of "is the `claude-pilot` launcher dead on this host?". Direct sibling of `live_pilot` above: one module, one verdict, two consumers — the dispatch gate in `validate_dispatch_readiness` and the Phase 2 filter in `auto_pull::classify_stuck_ready`. A predicate written twice is a predicate that can contradict itself, the lesson `grooming_marker` had to engrave once (mika#2158).

**What phase A left open.** Phase A (PR #2644) made the death **visible** — exit 79 stamped by the pre-flight, `pilot_launcher_dead` WARN, an `audit_events` row under `PILOT_LAUNCHER_HEALTH_TOOL`. It bounded **nothing**: every further dispatch left and died the same way until the host was repaired, which is the whole of the 2 h 45 measured on 2026-10-02. This module is AC2.

**A WINDOW, never a persistent counter (D5).** "≥ 2 `dead` rows in the last N minutes" rather than a consecutive-failure count, and the reason that decides it is that **the brake lifts itself with nothing to erase** — the property mika#2597 writes for its hold and mika#2347 had to build by hand for want of it. A persistent counter would need a reset site, and mika#2158 measured what a counter reset by the action it counts costs: 31 re-drives reading 1. Worst case is bounded and self-repairing: one dispatch burned per window instead of all of them.

**Fail-OPEN on the read (D6), and the asymmetry is the inverse of `wip_rescue`'s.** An unreadable ledger ⇒ **do not block**. There (mika#2199) and in `run_gh pr ready` (mika#2624) a false negative made **one** PR wait; here a false positive freezes **every** dispatch in the fleet. `LauncherHealth::is_braked()` exists so a caller cannot write `!matches!(v, Healthy)` and turn `Unreadable` into a block — the one inversion that would freeze the loop on a supposition, and the same guard, for the same reason, as `LivePilotVerdict::is_alive`.

**Three states, never a `bool`:** `Healthy` / `Braked { dead_count, since }` / `Unreadable { reason }`. The last two call for **opposite** conduct — "the launcher is broken, repair the host" against "the database is not answering" — and a boolean would make the second read as the first (the `HoldVerdict` motif, mika#2597). `classify_launcher_health` is pure and takes `Option<i64>`: `None` is "could not count" and is **never** `0` (mika#2331).

**Detection is unconditional; only the disposition is gated.** `MIKA_PILOT_LAUNCHER_BRAKE=0` disarms the refusal, not the measurement: the `dead` and `recovered` rows keep being written, and a brake that *would* have engaged while disarmed emits `pilot_launcher_brake_disarmed` (WARN, expected regime zero). Without that line, "the brake is disarmed" and "the host is healthy" would return identical bytes — class mika#2205 applied to the brake itself.

**The gate is bounded to the pilot dispatch skills** (`PILOT_DISPATCH_SKILLS`, mika#2649). `deploy_mika` reaches `validate_dispatch_readiness` too, and a dead `claude-pilot` launcher says nothing about a deploy — worse, the deploy is one of the gestures that **repairs** the host, so braking it would be the loop locking itself out of its own remedy.

**The `recovered` row is written on a TRANSITION only** (§16): a dispatch finishing without the dedicated code *while the window already carried a death*. One row per host repair, not one per dispatch — which is what keeps the volume nil in a healthy regime and satisfies the mika#2131 doctrine without an arbitration. `code() == None` (killed by a signal) counts as recovery, and correctly: the process had to have **started** to be signalled.

**The sole-writer scan was narrowed, and that is documented rather than silent.** `canonical_tokens::tests::mika2634_the_launcher_health_name_has_a_single_writer` compared the needle by **substring** and therefore counted `pilot_launcher_health_unreadable` — a *log event* name, built by prefixing the audit name — as a second writer of the audit row. Phase B made the comparison **exact**, the measured correction mika#2649 had to make on its own twin scan one ticket earlier, and added the good-faith control `mika2634_the_sole_writer_scan_still_catches_a_second_site` so that narrowing a guard comes with the evidence it still bites. Both allowlists stay shipped empty: when the scan fires, remove the second site (doctrine mika#2201).

Config keys, the six operator surfaces with their expected regimes, the three probes and their four halts: root `CLAUDE.md` § *Le frein : à la deuxième mort, le moteur cesse de dispatcher*.

**QA-review reconciler (mika#2334):** `qa_review_reconcile::reconcile_qa_review_requests()` — the fourth recurring scan, beside `auto_pull` and `wip_rescue`, carried by mika-dev on cron `0 */15 * * * *`. It asks `mika-platform-qa` for a review on the loop's open PRs that nothing came to review.

**The event it backs up is unique and nothing replays it.** `pull_request.opened` is what starts the QA cascade (routed by the gateway, no draft filter), so the pilot's death after the push cannot by itself prevent the review — the founding ticket's causal premise does not hold, and the trailing `gh pr edit --add-reviewer` it wanted moved **did not exist anywhere in the repo**. What does hold is that `opened` is losable in four places (drop-oldest in the mika#1870 bounded queue → 429 → the gateway circuit breaker → a DLQ row that only a manual replay pulls back; plus the empty LLM turn, where `webhook_zero_tools` is opposed once) and that **no path re-read an open PR without a review**. The one pre-existing recovery, the mika#1711 `check_suite.completed(success)` fan-out, requires `draft: false` **and** a green CI, so a PR whose CI is red or never ran was never caught.

**The decision is a pure function, and what is NOT an input to it is the point.** `select_prs_needing_review(prs, now, cfg)` takes no signal about how the pilot terminated — that absence *is* the structural form of "independent of pilot survival" (AC1), and it is why the fix covers the three upstream losses too, none of which are pilot deaths. Six conjunctive terms, each **fail-safe** (unreadable information takes a PR *out* of the population, never into it): open, author = `DISPATCHER_FORGE_LOGIN`, `isDraft == false`, no request **and** no review from `REVIEWER_FORGE_LOGIN`, age inside `]MIN_AGE, MAX_AGE[`. The review term is not redundant with the request term: GitHub withdraws the request once the review lands, so without it every reviewed PR would be re-requested for ever.

**Why not a step before the fragile ones, as the ticket's comment asked.** An *unconditional* gesture at PR creation doubles the review on **every** PR of the loop, not just the stranded ones: `opened` has already started a session, and the `review_requested` the gesture emits targets exactly `REVIEWER_FORGE_LOGIN`, so `is_suppressed_review_request` (mika#1655) does not filter it and a second session starts — one that cannot observe the first, since `gh pr view` is outside qa-review's tooled perimeter and the first review is not posted yet anyway. The gesture must therefore be **conditional on the absence of a review**, which the shell tail cannot be: `dispatch-lib.sh` delivers its callback and dies in seconds, and an absence is only measurable after a delay. A prompt step is ruled out by `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.

**No new literal.** `REVIEWER_FORGE_LOGIN` and `DISPATCHER_FORGE_LOGIN` both come from `mika_common::forge_identity`, which `mika-gateway` imports for the same filter — writing the login here would be the constant divergence a shell implementation would have guaranteed. **Token:** `Settings::resolve_github_token` (PAT-first, App fallback) via `resolve_periodic_scan_token`, covered by the mika#2205 structural guard; no label is written, so there is no `resolve_periodic_scan_label_token` here.

**SOLE WRITER:** `tool_name = 'qa_review_reconciled'`. That is what makes `SELECT … WHERE tool_name = 'qa_review_reconciled'` the exact list of PRs the loop had to catch up on — i.e. the measure of the nominal path's health. **Operator grep signals:** `qa_review_reconciled`, `qa_review_reconcile_tick` (aggregate, emitted **only when the tick acts**), `qa_review_reconcile_no_token`, `qa_review_request_failed` (should stay empty — `requested_reviewers` is the same family as the label write that already fails under PAT, mika#2228), `qa_review_reconcile_error`. Config and post-deploy probes: root `CLAUDE.md` § *réconciliation des demandes de revue*. **This is a net, not a path:** sustained volume means `opened` is being lost systematically and the net is now hiding the signal that would show it.

**Team task tree:** parent `invoke_orchestrator` task + child `resume_agent` tasks per delegation. Suspend/resume on pending grandchild callbacks. **Team-run user notification (#287):** fired once at terminal status from two symmetric callsites (`run_team` tool for sync completion, `dispatch_invoke_orchestrator` for async resume), both routing through `teams::notification::build_run_completion_message`. Per-child `resume_agent` callbacks have their user-facing `send_message` suppressed via `NoopSender`; the silent turn still runs (updates memory, records `llm_calls`) — only the user channel is gated. Deliverable text is UTF-8-safe truncated at 4000 chars (below Telegram's 4096 limit).
