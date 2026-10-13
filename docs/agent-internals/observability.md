# Observability

Moved verbatim out of `crates/mika-agent/CLAUDE.md` on 2026-10-13. That file is loaded whole into a pilot's prompt as soon as the pilot reads anything under `crates/mika-agent/`; at 594 KB it no longer fit, and `crates/mika-common/tests/claude_md_size.rs` now caps every `CLAUDE.md` in the repository. Headings keep their original level.


"Always instrument, optionally export" pattern. Two orthogonal correlation axes: `trace_id` (per-request/per-turn, 32-char hex) + `session_id`/`agent_id` (system-level). `unified_timeline` VIEW enables cross-subsystem queries.

**Span filtering:** Per-layer `filter::Targets` on the OTel layer exports only `target: "mika::otel"` spans (LLM calls, agent turns, server requests).

**LLM observability:** `llm_call` spans with `gen_ai.*` semantic convention attributes, feature-gated behind `#[cfg(feature = "telemetry")]`. When `log_llm_bodies=true`, request/response bodies are attached as `gen_ai.prompt`/`gen_ai.completion` span attributes for Langfuse Generation input/output (#671). Response bodies use `serialize_response_text()` from `mika-common::llm` (same format as `llm_calls.response_text`). Team engine emits `TeamEvent` variants for live dashboard updates.

**`turn_usage` says how big the brief was, including when the turn fails (mika#2331 AC1).** `TurnUsageFields` carries `request_bytes` and `system_prompt_bytes`, threaded through all three `build_turn_usage_fields` / `emit_turn_usage` pairs (the `Ok` arm, the `Err` arm, and `save_continuation_llm_call`). The `Err` arm is the point: `usage` is `None` there, so every token count is 0, and the line used to say nothing about the size of the brief that timed out — `input_tokens = 0` was all an operator measuring a 420 s hang could read. Both values are computed **before** the call (`request.payload_bytes()`, `system_prompt_len`) and therefore survive it; both were already written to `llm_calls` (v53 / v38) and were simply invisible to the log stream, so this moves an existing measurement from a gated surface to the ungated one. `Option`, and `null` is never `0`: no request is empty, so a zero would be a readable lie about a measurement that did not happen. Both are RAW dimensions, so Prime hard condition #1 (no `phase`/`is_planning`/`role`) still holds. Companion event `llm_call_attempt` lives in `mika-common` (see its CLAUDE.md); the procedure that reads the two together is in the workspace `CLAUDE.md` § *Lire un hang LLM « 420 s sans octet »*.

### The tail of the brief-size distribution is a population (mika#2474)

`brief_size_overrun` — an INFO event emitted at the **single** production site
where a conversation window is assembled (`agent_loop`, immediately after
`context_window_assembled`), **only** when the turn's `user_message_bytes` crosses
`Settings::effective_brief_size_alert_bytes()`. Under the threshold it is a no-op:
no line, no audit row.

**What the code reading displaced in the ticket, and it is the first deliverable.**
mika#2474 asked to *measure* and to *bound* the architect brief. **The measurement
already existed**, component by component, on ungated events: the system prompt on
`system_prompt_assembled` (`system_prompt_bytes`, `per_skill_bytes`, mika#1217);
the history, the plan and the tool definitions on `context_window_assembled`
(`history_bytes`, `user_message_bytes`, `tool_defs_bytes`, mika#2295); the total on
`turn_usage` (`request_bytes`, mika#2331). Nothing needed instrumenting — and a
fifth instrument restating those same numbers would have created two sources of
truth for one value, exactly what `emit_context_window_assembled`'s own doc-comment
warns against. **The one component carrying no ceiling is the plan**, and
`user_message_bytes` *is* the plan on an architect pass: `_arch_ask` runs
`mika ask … - < "$plan_path"`, so the file's content is the turn's user message.

**Its exemption from the ceiling is structural, and correct.**
`truncate_history_to_token_budget` stops at `history.len() - 1`, so the turn's own
user message is never elided. Truncating it would render a verdict on a mutilated
plan — **green and wrong**, which is strictly worse than a timeout, which is red
and visible (`ESCALATE`, `PIPELINE FAILURE`). Same arbitration mika#2368 had to
write when it refused to let a net post `pass`.

**So nothing is bounded, and the four conceivable transformations are refused with
their own motive:** truncating the plan (the verdict then covers a mutilated plan);
summarising it by an LLM call (the architect reviews a *summary*, and the summary is
itself a brief, so the cost moves rather than goes); splitting it into
independently-reviewed sections (the `docs/architecture/review-guide.md` findings —
SOLID / DRY / Orthogonality — are transverse by nature, so a per-section review
loses precisely what the review exists to find); and not re-sending the plan on the
second pass (the history *is* truncable — at `max_tokens = 8000`, a 72 KB plan is
elided **whole**, so the second pass would review nothing; the redundancy is real but
touches only *small* plans, i.e. not the population of the problem).

**Threshold, not ceiling — and the regime is deliberately non-empty.** 48 000 is the
**measured p90** of the 198 plans of September 2026 (p50 29 414, p75 37 382, p90
48 378, p99 72 349). The population is therefore the upper decile, ~2 lines a day,
which is why the event is **INFO**: a WARN on a nominal upper decile is what ends up
muzzled. The anomaly is not the line — it is the **correlation** between it and a
`turn_usage` carrying `status = "error"`, which is mika#2457's Halt 3 made
executable. Contrast `pilot_cost_overrun` (mika#2496), WARN because its threshold is
a posed *rule* whose crossing is a fault.

**Why a distinct event name rather than a flag on the sibling.** A boolean on
`context_window_assembled` buys nothing — reading it still means `jq` over *every*
turn, which `select(.user_message_bytes > 48000)` already does. What a name buys is
the **grep**: over 19 GB of log, the difference between a question and an analysis
(`llm_budget_resolved`'s reasoning, mika#2293; mika#2131's aggregate-in-log,
detail-elsewhere).

`system_prompt_bytes` rides on the line — `system.len()`, in scope because
`emit_system_prompt_assembled` consumed the prompt by reference — so the line is
**self-sufficient for attribution**: without it, "the brief was large" needs a second
grep to learn what it was large *of*. Not a second source of truth: the same value, at
the same instant, on the line that needs it.

**SOLE WRITER** of `brief_size_overrun` in the log and in `audit_events`
(`tool_name = 'brief_size_overrun'`, `target_key = 'agent:<id>'`, `after_value` = the
bytes **and nothing else** — it is what the operator averages, the
`pilot_cost_overrun` motif). Pinned by
`canonical_tokens::tests::mika2474_the_overrun_name_has_a_single_writer`, allowlist
shipped empty, with its anti-vacuity assertion. **No deduplication, deliberately**:
each crossing is a distinct dated fact one wants to *count*, the population is a few
a day rather than a hundred per tick, so mika#2131's doctrine does not apply
(`ready_label_outcome` motif, mika#2323). The audit write is fire-and-forget;
`brief_size_overrun_audit_failed` marks an INFO line whose row did not land, i.e. a
`GROUP BY` that undercounts.

**The threshold has ONE production reader**, held by
`mika2474_the_threshold_has_a_single_reader` (allowlist shipped empty, with a
good-faith control **verified red** on a real second site). A second reader would in
practice be the *refusal* mika#2474 declines to ship without its precondition, and it
would cut a groom on a value calibrated for an **alert**: a false alert costs one log
line, a false refusal costs an architect pass and a point of the mika#2020 re-drive
budget, three of which abandon a healthy ticket.

**Correction found on the way:** `ContextWindowFields::truncated_messages` /
`truncated_bytes` claimed "always `0` today" while brique 2 of mika#2295 had landed —
contradicted by the comment at the emission site in the same file. That matters
because `truncated_messages` is **the** field saying whether a second architect pass
had the first pass's plan elided out of its window, and an operator reading "always 0"
will not read the field.

**What this does NOT buy.** No verdict is made to fit under a ceiling; no size is
reduced; nothing is retro-filled (briefs already sent will never have their line —
fabricating an audit row dated to an unobserved crossing is the inverse of what this
defends). It makes the brief **legible**, not **watched**: the only new instrument is
a threshold, and its silence proves nothing until someone runs the probes. Config,
the four commands, the reading table and the four halts: root `CLAUDE.md`
§ *Optional (brief-size alert threshold — mika#2474)*.

**Session lifecycle:** Silent dispatcher variants call `end_session()` after completion. CLI commands call `end_session()` on all exit paths. `startup_recovery()` prunes old sessions via `prune_old_sessions()`.

### Guard Fabrication Telemetry (#953)

Structured telemetry for the five fabrication-class EndTurn guards. Each guard firing emits a `target: "mika::otel"` event (exported to Langfuse via OTLP when telemetry is enabled, always in log file). Two paired event types:

**Detection events** — emitted when a guard fires (step N):

| Event name | Guard position | Fires when |
|---|---|---|
| `guard.callback_state_claim` | 4c | Callback turn claims downstream state without `run_gh`/`check_task`/`gh_read` |
| `guard.fabricated_action_claim` | 5 | Claims action with GitHub URL but zero tool calls |
| `guard.dev_groom_fabrication` | 5b | Claims Verdict without `run_claude_pilot_groom` call |
| `guard.doctrine_public_promo` | 5c | Proposes / drafts a prohibited public-launch surface (Show HN, Product Hunt, Reddit launch, Twitter promo, growth-hack) in violation of Mika's invitation-only distribution doctrine (mika#1814) |
| `guard.false_local_hosting_claim` | 5d | Asserts this instance runs locally (or that the user's data never leaves their machine) while the resolved `Deployment` is not `Local` (mika#2290). Extra fields: `deployment`, `persona`, `matched_subject`, `matched_assertion` |
| `guard.response_language_drift` | 5f | Answers in a language other than the one this tenant declared via `customer_config.language` (mika#2247). Extra fields: `expected_language`, `detected_language`, `detected_hits`, `expected_hits`. Expected regime: **low but non-zero** — each line is a turn caught |
| `guard.time_of_day_greeting_mismatch` | 5g | Greets with a formula naming a part of the day the tenant is not in, while the local hour is known (mika#2247). Extra fields: `greeting`, `implied_part`, `actual_part` |
| `guard.testimony_access_proposal` | 5h **and** the tool channels (mika#2627) | Proposes to **open** access to testimony-grade data (Gmail / full Drive / personal journal) — a breach of the non-transit doctrine at the *propose* surface, with no tool call required for it to fire (mika#1960). Extra fields: `matched_subject`, `matched_movement`, and since mika#2627 **`channel`** ∈ `{end_turn, send_message, create_reminder, create_scheduled_task, team_deliverable}` (the fifth since mika#2633, whose lines also carry `team_run_id`, `team_name`, `deliverable_source`, and `writer_agent` when the provenance is `writer` — but **no** `agent_id`, the team DB scope not being the acting agent). Expected regime: **zero lines** on every channel. One name with the channel in a field (motif `ready_label_outcome`, mika#2323): the five populations share one cause and one operator conduct, so a `GROUP BY channel` sizes each. `guard_correlation_id`, `step` and `label` are **absent** on the tool channels — there is no re-prompt to join and the tool site knows neither (never fabricated: mika#2304, and `null` is not a value, mika#2331). **Dated cost:** a `jq 'select(.channel == "end_turn")'` spanning the mika#2627 deploy returns empty on older lines, where the field does not exist; they are not rewritten (mika#2361), so the query right on both sides is `select(.channel == null or .channel == "end_turn")` |
| `guard.asserted_unavailability` | 6c | Claims tool unavailable when it's in the enabled set |
| `guard.assert_grounded` | 6d | Affirms resource state without grounding tool call |

Fields: `trace_id`, `agent_id`, `session_id`, `step`, `guard_correlation_id` (`{trace_id}:{step}:{guard_label}`), `label` (mode), plus guard-specific detail fields.

**Correction event** — emitted when the next EndTurn passes all guards (step N+1):

| Event name | Fires when | Key fields |
|---|---|---|
| `guard.correction_accepted` | EndTurn accepted after a guard previously fired | `guard_correlation_id` (matches detection), `original_guard`, `original_step`, `corrected_content` (truncated to 2000 chars) |

**Residue event** — emitted when a single-retry budget is spent and the violation goes out anyway:

| Event name | Fires when | Key fields |
|---|---|---|
| `guard.false_local_hosting_claim_uncorrected` | Guard 5d already fired this turn and the re-prompted response still asserts local hosting (mika#2290) | `deployment`, `matched_subject`, `matched_assertion`, `step`, `label` |
| `guard.response_language_drift_uncorrected` | Guard 5f already fired and the re-prompted response is still in the wrong language (mika#2247) | `expected_language`, `detected_language`, `step`, `label` |
| `guard.time_of_day_greeting_mismatch_uncorrected` | Guard 5g already fired and the re-prompted response still names the wrong part of the day (mika#2247) | `greeting`, `implied_part`, `actual_part`, `step`, `label` |
| `guard.testimony_access_proposal_uncorrected` | Guard 5h already fired and the re-prompted response still proposes to open testimony-grade access (mika#1960) — **or**, on `channel = "team_deliverable"`, a team deliverable was withheld and the neutral line served in its place (mika#2633) | `matched_subject`, `matched_movement`, `channel`, plus `step`, `label` on the 5h residue and `team_run_id`, `team_name`, `deliverable_source`, `writer_agent` on the team residue |

**Expected regime: zero lines.** It exists because that population is otherwise indistinguishable from a healthy turn — the blind spot the Fire-Disposition gate (mika#1574) is there to close. It is **not** a second correction: the family grants one re-prompt, and a guard that diverged from its neighbours on that point would become the one everybody re-reads to find out why.

**Architect self-report** — complementary success signal (not fabrication detection):

When a verdict-producer agent (mika-arch) self-reports inability to anchor a citation (the desired behavior per #952 skill prompts), an `audit_events` row is written with `tool_name='fabrication_guard'`, `target_key='arch_anchoring_self_report'`. Queryable via SQL: `SELECT * FROM audit_events WHERE target_key = 'arch_anchoring_self_report'`.

**Query patterns:**
- Detection events: `grep 'guard\.' server.log | jq 'select(.event | startswith("guard.") and . != "guard.correction_accepted")'`
- Correction events: `grep guard.correction_accepted server.log | jq '{guard_correlation_id, corrected_content}'`
- Cross-event join: match `guard_correlation_id` between detection and correction events
- Self-report (SQL): `SELECT * FROM audit_events WHERE target_key = 'arch_anchoring_self_report'`

### Log Sinks

Two distinct sinks carry per-agent runtime log events (team runs have a third — see the table). Both emit the same structured JSON with an `agent_id` field on every entry — the difference is the file target and the process that writes to it.

| Sink | Initializer | Process | Path | Rotation |
|------|-------------|---------|------|----------|
| **Server log** | `mika_common::logging::init()` (`crates/mika-common/src/logging.rs:240`) | `mika-spirit` (long-running daemon) | `MIKA_SPIRIT_LOG_FILE` (e.g. `/var/log/mika/server.log`) | None — single file via `tracing_appender::rolling::never` |
| **Per-agent CLI log** | `mika_common::logging::init_pretty()` (`crates/mika-common/src/logging.rs:356`) | `mika-cli`, **for what still runs in-process**: `mika chat` (TUI) and its silent-mode background turns (heartbeat, reflection, reminder). **Not** `mika ask` agent turns since mika#1727 — see the carve-out below. (`mika team run` writes to the team sink, `~/.mika/teams/<team>/logs/`, via `init_team_logging` — `crates/mika-cli/src/main.rs:433`.) | `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` (daily) | Daily via `tracing_appender::rolling::daily` |

**`mika ask` carve-out (mika#1727, documented by mika#2069).** `mika ask` no longer runs the agent loop in-process. `crates/mika-cli/src/commands/ask.rs` is a thin client: it ships the prompt to the local mika-spirit daemon over A2A `message/send` at `{spirit_url}/a2a/{agent}` and renders the returned Task, with no in-process fallback. Spirit owns the execution session, so **every agent turn of a `mika ask` — `turn_usage`, `llm_call`, tool events — is emitted by the spirit process and lands in `MIKA_SPIRIT_LOG_FILE`**, not in the per-agent sink. The per-agent file is still initialized as that process's log sink (`crates/mika-cli/src/main.rs:233`), so the invocation's *client-side* events land there — the file is populated, just not with what a turn-accounting reader is after. Measured 2026-08-30 (mika#2069): 21 084 `turn_usage` in `/var/log/mika/server.log` against 346 across every per-agent log combined, and the 346 were all `heartbeat-…` / `reflection-…` / `reminder-…` / `mika chat` sessions — not one execution turn.

**Decision tree for operators:** If you want to read the running mika-spirit's runtime events (skill execution, task engine, callback lifecycle, autonomous-loop wedges) — **or any agent turn, including a `mika ask` one** — read `MIKA_SPIRIT_LOG_FILE` filtered by `agent_id`. Read `~/.mika/agents/<name>/logs/mika.log.<date>` for a `mika chat` session's turns, for silent-mode background turns (heartbeat, reflection, reminder), or for a `mika ask` invocation's client-side events. Both contain the same `agent_id` field on every entry, so cross-filtering by agent works in either sink.

**Single-sink rationale:** mika-spirit uses one file with `agent_id`-filtered queries instead of per-agent file appenders because: (a) per-agent appenders would double the disk-write rate per event, (b) they create a sync gap risk if the per-agent appender worker can't keep up, and (c) they duplicate data already correctly addressable via the JSON `agent_id` field. The per-agent CLI sink is correct for its purpose (discrete CLI invocations) and is not a substitute for the server log.

**Every event is one line — but only since mika#2195, and the cut-off matters when you read old data.** The JSON branch of `logging::init` used to compose *two* JSON layers, one on stdout and one on the file, while the OpenRC unit redirects stdout into that same file (`supervise-daemon mika-spirit --stdout /var/log/mika/server.log --stderr /var/log/mika/server.log`). Every event therefore landed in `server.log` **twice**, uniformly — not a `turn_usage` quirk but a property of the layer composition: `mika-spirit listening` (the server binds its port once) appeared twice per restart, `domain_rebuild_complete` four times per boot. **Any count taken over log lines written before the mika#2195 deploy is doubled** — that is the origin of RT-005's factor-2 (−7,43 → −3,71 deduplicated) and of mika#2179's "38 failures" that were 19. It never affected *values* inside an event, only how many times the event appeared, so deduplicating is the whole correction. The fix withholds the stdout layer whenever a log file is configured (`json_stdout_layer_enabled`, `crates/mika-common/src/logging.rs:226`); the price, stated: with `MIKA_SPIRIT_LOG_FILE` set, `docker logs` and `supervise-daemon --stdout` carry no application lines — read the file. Post-deploy probe: `grep -c 'mika-spirit listening' $MIKA_SPIRIT_LOG_FILE` over one restart window must be **1**, not 2. A 2 means the fix did not take — halt, do not redeploy blindly.

**Common mistake — and note the failure shape:** Audit tooling that reads `~/.mika/agents/<name>/logs/mika.log.YYYY-MM-DD` for server-mode events (or for agent turns of any kind since mika#1727) does **not** get an error, and does **not** get an empty file. It gets a sparse, plausible, wrong dataset: the heartbeat / reflection / reminder / `mika chat` turns that legitimately live there, which look enough like the real thing to be analysed by mistake. That is what mika#2069 caught in the RT-005 measurement channel. Use `jq 'select(.agent_id == "<name>")' < $MIKA_SPIRIT_LOG_FILE` for server-mode queries and for any turn accounting. `mika logs --agent <name>` prints both resolved paths.
