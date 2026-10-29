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
`system_prompt_assembled` (whose field is named **`total_bytes`**, not
`system_prompt_bytes` — the divergence is real, deliberate and documented in the
mika#2884 section below; plus `per_skill_bytes`, mika#1217);
the history, the plan and the tool definitions on `context_window_assembled`
(`history_bytes`, `user_message_bytes`, `tool_defs_bytes`, `system_prompt_bytes`
since mika#2884, mika#2295); the total on
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

### `tool_defs_bytes` is a total; the arbitration is name by name (mika#2783)

`tool_defs_census` — an INFO event (`target: "mika::otel"`) emitted at the
**single** production site where a conversation window is assembled (`agent_loop`,
immediately after `context_window_assembled`, before `report_brief_size_overrun`),
on the **same** `&skill_tool_defs`. It carries `per_tool_bytes`, a JSON
`{name: bytes}` map, plus `tool_defs_bytes`, `tool_defs_count`, `per_tool_sum`
and the four correlation fields (`agent_id`, `session_id`, `trace_id`, `mode`).

**What was missing, and who needs it.** `context_window_assembled` already carried
`tool_defs_bytes` since mika#2295 — *a total*. mika#2593 had to transport its
proportion from the source instead, a JSON serialization done by hand at
implementation time (4 252 bytes for nine definitions, 899 for
`promote_deferred_callback` alone), stated as a **prediction**, with the total
remaining the only post-deploy arbiter. That was tolerable because mika#2593's
criterion was *structural* — C1 ∧ C2 ∧ C3, "is this reading orphaned?" — and a
structural question is not weighed. **mika#2748 arbitrates name by name**:
`MIKA_ARCH_ALLOWED_TOOLS` is a list of 19 names, and every future request to add a
twentieth is a budget question asked about **one** tool. The only answer available
was "the total moved by so much after deploying", which arrives too late to decide
and mixes the added name's contribution with every other variation of the same
turn. The sum existed; the detail was thrown away.

**Why a distinct event rather than a field on the sibling.** It must grep alone
(the `brief_size_overrun` reasoning above, and `llm_budget_resolved`'s, mika#2293):
the census is read by someone arbitrating one name, and that reading is a `jq` over
one event per turn, not a filter over the busiest line of the stream. A field on
the neighbour would also widen a line every conversation turn already pays for, to
serve a question asked a few times per allowlist change.

**Why one map and not one line per tool, and not "first assembly only" either.**
≈ 50 lines per turn per agent is the volume the ticket named; one line carrying
`{name: bytes}` answers the same question at 1/50th of the stream. The
first-assembly form is refused by a fact of the code: **the served array is not
constant per agent.** Matched skills vary by turn, the denylist widens on a Webhook
Fallthrough (mika#2517), MCP tools come and go, and `filter_ci_excluded_tools`
removes names on a CI prefix. A first-assembly-only emission would describe one
turn and be read as describing the agent. Form precedent is one function away:
`emit_system_prompt_assembled`'s `per_skill_bytes`, the same shape for the other
half of the brief.

**Why INFO.** A census of the nominal case fires on every turn by construction, and
a WARN on a nominal regime is what ends up muzzled (`brief_size_alert_bytes`, same
arbitration). The three WARNs below are the non-nominal arms and they **add** to
the INFO line, never replace it.

**The equality is the ticket's arbiter**, and it is asserted rather than assumed:

```
per_tool_sum + json_array_framing_bytes(tool_defs_count) == tool_defs_bytes
json_array_framing_bytes(n) == 2 + max(n - 1, 0)
```

`serde_json` compact-serializes `[e₁,e₂,…,eₙ]`: two brackets plus `n − 1` commas,
hence the saturating subtraction — `[]` is `2`, and a count of `0` must not wrap to
`usize::MAX`. The total passed in is **the value the sibling published**
(`context_window_fields.tool_defs_bytes`), not a second call to
`tool_defs_byte_weight`: a recomputation would be free to agree with itself. A
divergence emits `tool_defs_census_mismatch` (WARN) with a **signed** `delta`,
because the sign is the diagnosis — short means a definition failed to serialize
(one weighs `0`), long means the two sites no longer weigh the same array. Without
the equality the census and `tool_defs_bytes` would be two populations under one
reading (class mika#2205), which is exactly what this instrument exists to close.

**Duplicate names are summed, and said.** The MCP append at the assembly site does
not deduplicate against the builtin and skill names, so two definitions can share
one name. The map holds their **sum** under one key, so the equality holds by
construction whatever the array contains; `tool_defs_census_duplicate_name` (WARN,
with `tool` and `occurrences`) reports the collapse, because `per_tool["x"]` then
answers "what do the x's cost together" rather than "what does x cost". Note
`tool_defs_count` is `tool_defs.len()`, **never** `per_tool.len()` — the framing
arithmetic is a function of the former.

**Absent is never zero — at three distinct levels, and they are not the same
reading:**

| what is absent | what it means | how it is held |
|---|---|---|
| **a key** of `per_tool_bytes` | that tool **was not served** to this agent on this turn | property of the map: a tool absent from the array has no entry. Never `0` (mika#2331) |
| **the whole line** | the binary in service **predates** this change | the emission is unconditional and ungated (mika#2340) |
| **the `per_tool_bytes` field** | never happens | it degrades to `"{}"` at worst — an empty array is `{}` with `tool_defs_count = 0`, never a mute line |

The pair `(per_tool_bytes = "{}", tool_defs_count = n)` therefore separates a
genuinely empty array (`n = 0`) from a serialization failure (`n > 0`), and the
latter carries its own WARN, `tool_defs_census_unserializable`.

**SOLE WRITER** of `tool_defs_census`, held by
`canonical_tokens::tests::mika2783_le_nom_de_levenement_a_un_seul_scribe` — a
source scan over all of `crates/`, needle composed at runtime so the guard file
does not denounce itself, comment lines stripped, with its anti-vacuity assertion.
**No exception list, and no constant in which to open one** (doctrine mika#2201 /
mika#2323: an allowlist born empty is a place to deposit the next infraction);
pre-existing population is zero, measured. A second writer would publish a second
`{name: bytes}` map for the same turn, indistinguishable in the log. When it fires,
route the site through `agent_loop::TOOL_DEFS_CENSUS_EVENT` or remove it.

**Scope: conversation mode, which is where the magnitude is.**
`emit_context_window_assembled` has **one** caller; the silent and team paths do
not build `ContextWindowFields` and therefore never compute `tool_defs_bytes`. The
`mode` field travels anyway, so a future extension to the other two modes needs no
schema change. The architect passes — the ticket's driving population — are
conversation turns (`_arch_ask` → `mika ask`), so the scope covers the need.

**What this does NOT buy.** No tool is removed, no array is bounded, nothing is
refused, and nothing is retro-filled: turns already served will never have their
line. It makes one name's cost **readable**, not **watched** — the probes below are
what make it true, and the instrument is not live until `make deploy`.

**Post-deploy probes**, all on `$MIKA_SPIRIT_LOG_FILE`:

```bash
# P1 — the instrument exists. No line ⇒ the binary predates the change
#      (mika#2340), NEVER "no tool served".
grep tool_defs_census "$MIKA_SPIRIT_LOG_FILE" | tail -1 \
  | jq '{agent_id, mode, tool_defs_count, per_tool_sum, tool_defs_bytes}'

# P2 — the equality holds in production. THE arbiter. Expected: empty.
grep tool_defs_census "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.tool_defs_bytes
      != (.per_tool_sum + 2 + (if .tool_defs_count > 0 then .tool_defs_count - 1 else 0 end)))
      | {agent_id, trace_id, tool_defs_bytes, per_tool_sum, tool_defs_count}'
# …and its positive control, which says the probe is looking at something:
grep -c tool_defs_census_mismatch "$MIKA_SPIRIT_LOG_FILE"    # expected: 0
grep -c '"event":"tool_defs_census"' "$MIKA_SPIRIT_LOG_FILE" # expected: > 0

# P3 — the question this ticket exists for: what does ONE name cost?
grep tool_defs_census "$MIKA_SPIRIT_LOG_FILE" \
  | jq -r 'select(.agent_id == "mika-arch") | .per_tool_bytes | fromjson
           | to_entries[] | "\(.value)\t\(.key)"' \
  | sort -rn | uniq | head -20

# P4 — negative control: absent is not zero. Expected: false on mika-arch
#      (promote_deferred_callback is in MIKA_ARCH_DISABLED_TOOLS, mika#2593),
#      and true on an agent that receives it (mika-dev). The PAIR is what
#      separates "not served" from "mute instrument".
grep tool_defs_census "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent_id == "mika-arch") | .per_tool_bytes | fromjson
           | {promote_deferred_callback: (has("promote_deferred_callback"))}' | tail -1

# P5 — no duplicate name. Non-zero ⇒ an MCP server serves a name already taken;
#      the equality still holds, but that name's value aggregates two definitions.
grep -c tool_defs_census_duplicate_name "$MIKA_SPIRIT_LOG_FILE"   # expected: 0
```

**Session lifecycle:** Silent dispatcher variants call `end_session()` after completion. CLI commands call `end_session()` on all exit paths. `startup_recovery()` prunes old sessions via `prune_old_sessions()`.

### The attribution line carries every component of the window (mika#2884)

`context_window_assembled` gains `system_prompt_bytes`, a bare `usize` emitted
next to the other three byte counts rather than at the end of the field list —
`history_bytes`, `user_message_bytes`, `tool_defs_bytes` and
`system_prompt_bytes` are the four components of the window and are read
together. Full field set: the four byte counts, `message_count`,
`distinct_sessions`, `history_scope`, `oldest_age_secs`, `truncated_messages`,
`truncated_bytes`, and the four correlation fields (`agent_id`, `session_id`,
`trace_id`, `mode`).

**What it buys is a join avoided, not a measurement saved.** The driving ticket
stated that the size of the system prompt "exists only on `turn_usage`" and is
"lost" when a turn dies before it. That premise is **inexact on the value and
exact on the name**: three surfaces already carried it, at different coverage.

| surface | field | when | coverage |
|---|---|---|---|
| `system_prompt_assembled` (mika#1217) | **`total_bytes`** | **before** the model call | every turn, unconditional |
| `brief_size_overrun` (mika#2474) | `system_prompt_bytes` | before the model call | only on a threshold crossing |
| `turn_usage` (mika#1889 / mika#2331) | `system_prompt_bytes` | **after** the model call | turns that reach a response |

So the "over-long prompt refused on the first turn" scenario never erased the
measurement — `system_prompt_assembled` is emitted before the call and carries
it, under another name. What the ticket was right about is the **name**
(`grep system_prompt_bytes` does not find `total_bytes`) and the **join**:
reading what filled one window meant correlating two or three events by
`trace_id`. Now one line answers it.

**Why a field on the sibling here, when this document refuses one twice.**
mika#2474 refused an `oversized` boolean on this very event, and mika#2783
refused the per-tool census as a field on it (*"it must grep alone"*). Both
refusals concerned a **new measurement needing its own grep**. This need is the
inverse: the value must *not* grep alone — it already greps on three lines — it
must be **on the attribution line**. The governing precedent is mika#2474's own
§ *`system_prompt_bytes` rides on the line*, which the doc-comment of
`report_brief_size_overrun` phrases as *"the same value, at the same instant, on
the line that needs it"*. Same argument, applied to the line that decomposes the
window.

**And it is not a second source of truth** (the warning a few sections above):
all emissions derive from `system.len()` on the same `String`. The equality holds
by construction, on three readings of it — `system.len()` at the call site,
`request.system = Some(system)` on its way to the provider, and
`request.system.as_ref().map_or(0, |s| s.len())` for `turn_usage` — plus two
properties: nothing mutates `system` between its assembly and its move into
`LlmRequest`, and `system_prompt_len` is computed **before** the step loop, hence
before the end-of-budget nudge that does `system.push_str(nudge)`. That second
property is what makes the equality non-trivial: on a multi-step turn with a
nudge, a naive `turn_usage` would have diverged. It does not, and
`tests/eval/test_context_window_system_prompt_bytes_2884.rs` pins both halves
through a real turn (the field equals the bytes actually sent; and it equals what
`turn_usage` reports for the same turn).

**Scope, unchanged:** `emit_context_window_assembled` still has **one** caller
(see the mika#2783 note above) — the silent and team paths do not build
`ContextWindowFields` and emit nothing here, so there is nothing to carry there.
`system_prompt_assembled`'s `total_bytes` is **not** renamed: that would be a
schema change on an event operator probes have grepped since mika#1217, and the
ticket said "no other change". The divergence is documented rather than resorbed.
No threshold and no alert is added on the new value either — `brief_size_overrun`
is already the threshold detector for this measurement, and a second one would be
the duplicate truth this document warns against.

**Post-deploy reading.** After `make deploy`, and not before: the instrument is
not alive while the binary in service predates the change (mika#2340 motif).

```bash
# The full decomposition of a window, one line, no join.
grep context_window_assembled /var/log/mika/server.log \
  | jq -r 'select(.fields.agent_id == "mika-dev")
           | [.timestamp, .fields.system_prompt_bytes, .fields.history_bytes,
              .fields.tool_defs_bytes, .fields.user_message_bytes,
              .fields.message_count] | @tsv'
```

What becomes readable on it: the **share of the system prompt** in the window,
turn by turn, without touching `turn_usage`; a **jump in `system_prompt_bytes` at
constant history**, which is the signature of a repository change (`CLAUDE.md`, a
bundled skill) rather than a conversation change; and **turns that died before
`turn_usage`**, which now show their prompt size on the attribution line instead
of requiring a second grep on `system_prompt_assembled` and its divergent field
name.

**An absent field on this line means "binary older than this change"**, never
"empty system prompt": at the emission site the prompt is a `String` in hand, so
zero is not a reachable state for a turn that emitted the event (mika#2331).

### Exhausting the step budget names the tools that consumed it (mika#2877)

`max_steps_tool_census` — a WARN event (`target: "mika::otel"`) emitted at the
**single** site where the agent loop cuts a turn at `MAX_TOOL_STEPS`, immediately
after the pre-existing `"agent exceeded max tool steps"` WARN and before
`all_tool_summaries` is moved into `LoopResult::MaxStepsExceeded`. It carries
`per_tool` — a compact JSON `{name: calls}` map under sorted keys — plus
`total_calls`, `steps_with_tools`, `distinct_tools`, `failed_calls`, `max_steps`,
the same `label` as the neighbouring WARN, and the three correlation fields
(`agent_id`, `session_id`, `trace_id`). Module: `agent_loop::max_steps_census`.

**What was missing.** The neighbouring WARN carries `label` and `max_steps` only:
it says *that* the budget was exhausted and *what* the ceiling was, never *by
what*. Measured 2026-10-22 on `/var/log/mika/server.log`: 204 occurrences in ten
days, almost entirely on mika-qa, and the three candidate corrections — raise the
ceiling, fix a looping tool, shorten the QA prompt — are decided exactly on this
data. This is a measurement flight: neither the ceiling nor the behaviour at
exhaustion changed.

**Why a distinct event rather than fields on the neighbour.** The finding above
*is* a grep of that line, so the line is the denominator of the measurement, and
adding fields would rewrite the very line the series follows. A neighbouring event
leaves the old curve readable, starts the new one at the deploy date, and lets the
two be compared.

**Why WARN.** To travel with the WARN it accompanies: an exhaustion is abnormal,
and an operator filtering their log at WARN must see both lines or neither. This
is the opposite arbitration from `tool_defs_census` above, and for the opposite
reason — that one fires on every nominal turn, this one only on a cut.

**`total_calls` is not bounded by `max_steps`.** A step carries as many calls as
the model emitted `tool_use` blocks, so twenty steps can hold more than twenty
calls. `steps_with_tools` is therefore load-bearing, not decorative: without it
`total_calls = 20` covers two non-subtractable populations (class mika#2205) —
"20 steps × 1 call" (the model is working, the ceiling is too low) and "5 steps ×
4 parallel calls" (the budget burns elsewhere) — which call for opposite
corrections.

**Failed calls stay counted in `per_tool`.** A failure consumes budget like a
success; excluding them would erase exactly the "a tool looping on its error"
pattern. `failed_calls` is the separate count, never a subtraction from the map.

**`max_steps` is `20` OR `30`, per agent, since mika#2926.** The ceiling is
resolved at a single site — `planning::policy::max_tool_steps_for_agent(agent_id,
mode.max_steps())` at the top of `run_loop` — and mika-qa is the only agent it
answers `30` for. So **grouping a census series by `max_steps` alone mixes two
populations** (class mika#2205): on a mika-qa line `30` is nominal, on any other
agent's line it would be a bug. Group by `(agent_id, max_steps)`, or read
`max_steps` only after filtering on `agent_id`. The field is the ceiling the loop
**served**, transported out of the loop body by `LoopResult::MaxStepsExceeded`'s
`max_steps` field, never recomputed downstream from a constant or from the
trigger — which is why the silent-path WARN below now carries the same value as
this line, where it previously carried the trigger's declared `20`.

**Absent is never zero — at three distinct levels:**

| what is absent | what it means | how it is held |
|---|---|---|
| **a key** of `per_tool` | that tool **was not called** on this turn | property of the map: the census enumerates what consumed, not the universe of tools. Never `0` (mika#2331) |
| **the whole line** | the binary in service **predates** this change | the emission is unconditional and ungated (mika#2340). **Never** "no tool was called" |
| **a non-empty map** | nothing: `per_tool = {}` with `total_calls = 0` is **a measurement** — the budget burned outside tool calls (guard re-prompts), and that may be *the* answer |

The last row is why the call site carries no `if !summaries.is_empty()`: such a
guard would mute precisely the most interesting case.

**Operator reading** (`/var/log/mika/server.log` on the host, or
`$MIKA_SPIRIT_LOG_FILE`):

```sh
# The tools that consume the budget, by decreasing cost, on mika-qa
grep max_steps_tool_census /var/log/mika/server.log \
  | jq -r 'select(.agent_id == "mika-qa") | .per_tool' \
  | jq -s 'reduce (.[] | fromjson | to_entries[]) as $e ({}; .[$e.key] += $e.value)
           | to_entries | sort_by(-.value)'

# Parallelism or length: total_calls vs steps_with_tools
grep max_steps_tool_census /var/log/mika/server.log \
  | jq -r '[.label, .total_calls, .steps_with_tools, .distinct_tools, .failed_calls] | @tsv'
```

**Conformance probe — a DIFFERENCE, not an equality (corrected by mika#2926):**

```sh
tot=$(grep -c "agent exceeded max tool steps"        /var/log/mika/server.log)
sil=$(grep -c "silent agent exceeded max tool steps" /var/log/mika/server.log)
cen=$(grep -c max_steps_tool_census                  /var/log/mika/server.log)
echo "$((tot - sil)) == $cen ?"
```

**Why the subtraction.** `"agent exceeded max tool steps"` is a **substring** of
`"silent agent exceeded max tool steps"`, and a cut silent turn emits *both*
lines: the first inside `run_loop` (`mod.rs:5194`, next to the census), the second
in `run_silent_agent` after the loop returns (`mod.rs:7717`). So a cut on the
callback or heartbeat path contributes **two** to `tot` and **one** to `cen`, and
the equality form over-counts by exactly the number of silent cuts. Measured on
the mika#2926 window: 13 census lines, 4 of them on callback turns, so the
equality form read `17 ≠ 13` and would have been diagnosed as *"a cut path
escapes the emission"* — the opposite of the truth. `tot - sil` is the count of
**loop-body** cuts, which is the population the census enumerates.

Census and loop-body WARN are emitted at the same site without condition, so
`tot - sil == cen` holds by construction on any binary postdating mika#2877. An
inequality is not a log detail: **left > right** means a cut path escapes the
emission (or the log spans the deployment); **right > left** means a double
emission, which the harness test forbids
(`tests/eval/test_max_steps_tool_census_2877.rs` asserts *exactly one* event per
turn — a turn emitting two would make the reading above ambiguous without failing
anything). All three at `0` means no exhaustion over the window.

**What this did NOT buy, and what read it.** mika#2877 changed neither the
ceiling nor the behaviour at exhaustion, and turns already served will never have
their line. **That second flight is mika#2926**, decided on this very series: 13
cut mika-qa turns in 3.5 days, `run_shell` taking 9 to 28 of them, `qa_pr_view`
called once. It raised mika-qa's ceiling to 30 *and* bounded `run_shell`
exploration in both review prompts — the measurement is what said a raise alone
would displace the exhaustion rather than remove it. Its own post-deploy probe is
this counter's fall, and lives in
[`docs/operator/environment-variables.md`](../operator/environment-variables.md)
§ *Le budget de pas d'un tour de revue*.

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
| `guard.response_cjk_script` | 5i | Answers in Chinese ideographs (U+4E00–U+9FFF) rather than the Latin script — at least 5 ideographs **and** at least 30 % of the script characters (mika#1680). Extra fields: `ideographs`, `latin`, `share_pct`. Measured population: 5 mika-dev assistant responses of 15 on 2026-10-29, under `z-ai/glm-5.2`. Expected regime: **low and falling** — the prompt line of `write_latin_script_line` is what should get the right answer first try; this guard is what measures whether it does. The range is **exactly** the one the AC4 probe globs (`content GLOB '*[一-鿿]*'`): guard and probe must count the same population, or the probe's target of 0 says nothing about the guard. CJK punctuation (U+3000–U+303F), kana and Hangul are deliberately out of the predicate. **Appended at the tail of the chain rather than next to 5f**, which would read better and would cost renumbering 5g and 5h across eight sites; the order is observably neutral because every earlier guard's lexicon is Latin and the budgets are per label |
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
| `guard.response_cjk_script_uncorrected` | Guard 5i already fired and the re-prompted response is still written in Chinese ideographs (mika#1680) | `ideographs`, `latin`, `share_pct`, `step`, `label` |
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
