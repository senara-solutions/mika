# mika-dev work assignment — the two gates, and what a silence means

> Doctrine record for mika#1745 (relocated from `mika-platform#185`, 2026-07-08).
> The code lives in `crates/mika-agent/src/server/ci_failure_handler.rs`.

## 1. The question this document answers

When a GitHub event reaches mika-dev, two questions decide what happens to it,
and they are **not** the same question:

1. **Is this repository mine to act in?** — a structural fact read off the event.
2. **Is this work mine, i.e. did I start it?** — a correlation against the task
   ledger.

The defect mika#1745 was filed against is that mika-dev only ever asked the
second, and answered a *no* with silence. In his own words (2026-07-07, the first
instance of the `family-inside-view-overturns-outside-read` bearing):

> "The task system is my only mechanism for 'is this mine?' — and it only tracks
> work I originated. It can't track work I should own but didn't dispatch. So I'm
> mechanically correct per my wiring, and mechanically blind to a significant body
> of work in my own repos."

The fix is **surface-for-adoption, not auto-adopt** (Prime's sharpening, persisted
2026-07-07 13:15 CEST): the gate becomes a decision point, not an automatic
action.

## 2. Gate 1 — repository ownership. Two lists, and they decide different things

| repository | `INTERNAL_REPOS` (gateway) | `DISPATCHABLE_REPOS` (agent, mika#2046) |
|---|---|---|
| `senara-solutions/mika` | ✅ | ✅ |
| `senara-solutions/mika-cloud` | ✅ | ✅ |
| `senara-solutions/mika-skills` | ✅ | ✅ |
| `senara-solutions/mika-platform` | ✅ | ✅ |
| `senara-solutions/claude-pilot-py` | ✅ | ❌ |
| `senara-solutions/wizzard` | ✅ | ❌ |

- **`INTERNAL_REPOS`** (`crates/mika-gateway/src/github.rs`) decides which
  repositories **route** to mika-dev without a `github_repos` row.
- **`DISPATCHABLE_REPOS`** (`crates/mika-agent/src/webhook_dispatch.rs`) decides
  where the loop may **dispatch**. The 2026-08-29 operator decision keeps
  `claude-pilot` and `control-monitor` spawn-CC-only; `wizzard` is a read-write
  controlled repo the loop has never driven.

**The ownership list of AC1 is the second one**, and the reason is not
conservatism. `surface_for_adoption` proposes an action — *create a task from this
event and engage* — and `DISPATCHABLE_REPOS` is the list that answers whether the
proposed action is **executable**. Surfacing `claude-pilot-py` would put a
proposal in front of the operator that the mika#2046 tool-boundary gate refuses
structurally. That repository therefore belongs to **AC4 (silent-stop)**, not to
AC1.

**There is deliberately no third list.** A third would be the programmed
divergence this repo has already paid for twice — dispatch seats (mika#2092,
guarded both ways by `scripts/check-dispatch-seats-declared.sh`) and
`DISPATCHABLE_REPOS` ↔ `.github/labels.yml`.

## 3. Gate 2 — task correlation, and where it lives now

The ticket locates the gate in `mika_agent::skills::self-dev-webhook-*`, at the
decision-table rule *"No matching task found → STOP"*. That line still exists
(`skills/bundled/self-dev-webhook-ci/system_prompt.md`), **but since #594 it is
not what decides**: `server::ci_failure_handler` intercepts
`check_suite.completed(failure|timed_out)` **before the LLM turn**, and the
silence was posed there — `Passthrough { enrichment: None }`, i.e. the model
receives the raw webhook text with not a word from the engine, then its prompt
tells it to ignore it.

**Consequence for any future fix in this area.** A remedy written in the prompt
would not hold: `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
(mika#2120 — nine recurrences under prompt enforcement against zero when the fact
is posed by code). The half that holds is written by the **engine**, before the
turn. The prompt carries the intent half and nothing more.

## 4. Surface-for-adoption — what it is and what it refuses to be

Three surfaces, and only the first two hold whatever the model does:

| surface | written by | holds if the LLM ignores everything? |
|---|---|---|
| `audit_events` row | the handler, before the turn | **yes** |
| operator notification (`send_message`) | the handler, before the turn | **yes** |
| `enrichment` on the `Passthrough` | the handler, read by the LLM | no — the intent half |

**`Passthrough`, never `Handled`.** `Handled` replaces the message and tells the
LLM an action was taken. AC3 forbids automatic adoption, so the turn must trigger
nothing: `Passthrough { enrichment: Some(…) }` lets the turn run exactly as it
runs today, with one more fact in front of the model.

**Detection is unconditional; only the disposition is gated** (motif
mika#2249/#2272). The audit row and the log line are the measurement and stay
readable when an operator cuts the noise; the notification sits behind
`MIKA_SURFACE_FOR_ADOPTION` (default armed — see `.env.example`). Disarmed is an
**observation mode**, not a blindfold.

**Every term is fail-safe towards today's silence.** A repository that is not
dispatchable, an unreadable `head_sha`, a database that will not answer the dedup
query — each falls back on the pre-mika#1745 behaviour. The asymmetry that
justifies shipping this armed: a missed surface costs an event a human does not
see (visible, recoverable); a spurious surface costs one log line. Neither error
is destructive.

**Dedup is a viability condition, not a refinement.** One push produces up to
**8** `check_suite.completed` events, one per workflow (measured by mika#1869,
which closed the same class on the success side). Key:
`pr:{repo}#{n}@{head_sha}` — the same form `ci_success_handler` and
`qa_review_reconcile` already use. Window: one hour. Fail-open: an unreadable
`audit_events` lets the surface through, the inverse of `wip_rescue`'s
fail-closed read of the same primitive, because there a replay cost the whole
queue and here it costs a line.

**`surface_for_adoption` is SOLE WRITER** of its name, in the log and in
`audit_events`, held by
`canonical_tokens::tests::mika1745_the_surface_name_has_a_single_writer`
(allowlist shipped empty). That property is what makes
`SELECT count(*) … GROUP BY target_key` exact rather than a number two sites can
disagree about — and **that count is the explicit precondition** of the
auto-adoption decision AC3 defers ("until we have enough n").

## 5. The doctrine, in one line each

- **An event in a repository outside `DISPATCHABLE_REPOS` stops, silently but
  said.** The `surface_for_adoption_skipped` line with
  `reason = "repo_not_dispatchable"` exists because a silence nobody can
  attribute is what this whole ticket exists to end — and that includes the
  legitimate silences.
- **An event in a dispatchable repository with no matching task is surfaced, not
  adopted.** No task is created, no dispatch is triggered, no status is written,
  no process is signalled.
- **The operator ratifies adoption per instance.** The remedy the notification
  names is the existing gesture: put `ready` on the linked issue, or dispatch by
  hand.

## 6. The ticket's three verification cases, with their real status (AC-R)

Written down so a future reader does not rediscover R2 and R3 at their own
expense.

| ticket's case | real status | why |
|---|---|---|
| **1 — a cpp Dependabot QA-pass event should surface** | **AC4, not AC1** — it must stay silent, on two counts | `claude-pilot-py` is deliberately outside `DISPATCHABLE_REPOS` (2026-08-29 operator decision), so surfacing it would propose an unexecutable action. And the *Dependabot* half is already closed since mika#1729, more strongly than the ticket asks: on a dispatchable repo a task-less Dependabot PR **merges** through the `pass` path, with `pr_merge_with_gate` as the hard guard (see `self-dev-webhook-qa/system_prompt.md`, box *Task-less Dependabot PRs*). |
| **2 — a CI success on an owned repo it did not dispatch** | **already covered**, no silence to lift | `ci_success_handler` handles a missing task without stopping: the `MergeReadySignal` is emitted with `task_id = "none"`. |
| **3 — an event on an UNowned repo should still silent-stop** | **held, and now attributable** | Same arm as before; it now writes one `surface_for_adoption_skipped` INFO line naming the motif, so the silence can be told from a broken predicate. |

**What is left, after rectification:** one real hole — *a CI failure, on a
dispatchable repo, with an open PR, and no task.* That is the population this
mechanism covers, and it is the only one.

## 7. Operator surfaces

```bash
grep surface_for_adoption "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{event, repo, pr_number, branch, head_sha, dedup_skipped, notified}'
```

```sql
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'surface_for_adoption' GROUP BY 1 ORDER BY 2 DESC;
```

| surface | expected regime | reading |
|---|---|---|
| `surface_for_adoption` (`dedup_skipped: false`) | **non-empty, low** | each line is legitimate work the loop used to ignore |
| a `target_key` counted `> 1` | **anomaly** | the dedup is not biting |
| `surface_for_adoption_skipped`, `reason = repo_not_dispatchable` | non-empty | AC4 doing its job — a legitimate silence, now attributable |
| `surface_for_adoption_skipped`, `reason = unreadable_head_sha` | **empty** | no dedup key was derivable; the event left the population |
| `surface_for_adoption_audit_failed` | **empty** | the log line landed and the audit row did not — the `GROUP BY` undercounts |

The grep is a **prefix** match, so it returns the signal and its skip siblings
together; `.event` discriminates. Filter on `.event == "surface_for_adoption"` for
the signal alone.

## 8. Post-deploy probes, and their four halts

> **Preamble.** `skills/bundled/` is a projection of the **binary**, not of the
> checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer` must carry the sha
> you just built, or every probe below describes yesterday's binary.

**S1 — the surface bites (first CI failure with no task).** One line, one
notification, no task created.
*Halt 1 — no line while such an event did occur:* do not widen the predicate by
reflex. Establish first that the served binary carries the fix (class mika#2340),
then read the `repo_not_dispatchable` motif — the repository may legitimately be
out of scope, which is AC4 working.

**S2 — sizing (30 days).** **This measurement was not available to the
implementer**: `~/.mika/data/mika.db` is not readable from the dispatch sandbox,
so the real volume of task-less CI failures is **unknown** at delivery. It is an
operator gesture on the host.
*Halt 2 — the count carries nominal traffic* (several a day, on different PRs):
it is not the predicate that is too wide — a significant share of the work in
dispatchable repositories does not go through the pipeline, **and that is the
result the ticket was looking for**. Note the count, disarm the notification
(`MIKA_SURFACE_FOR_ADOPTION=0`) to keep the measurement without the noise, and it
is that count which opens the auto-adoption decision AC3 defers.

**S3 — negative control for noise (7 days).** No surface on an event whose PR
carries a task.
*Halt 3 — one occurrence:* the task correlation is broken upstream
(`find_active_task`: `pr_url` then branch) and **that** is where to look, not in
the dedup window.

**S4 — AC4 holds.** Zero surface on `claude-pilot-py` and `wizzard`.
*Halt 4 — one occurrence:* the predicate is reading `INTERNAL_REPOS` instead of
`DISPATCHABLE_REPOS`. Disarm, fix the list being read — do not add a downstream
filter.

**Transverse halt — both probes mute.** Zero surface **and** zero skip proves
nothing: a task-less CI failure must actually have happened since the deploy.
*A guard nobody has exercised reads exactly like a guard that works* (mika#2205).

## 9. What this does NOT buy

- **It adopts nothing.** That is AC3, and it is the point of the ticket: the gate
  becomes a decision point, not an action.
- **It covers one event path.** The CI-failure one. Other task-less webhook gates
  (`pull_request.closed` without correlation, `issue_comment`) keep their
  behaviour — an unmeasured population, and instructing before measuring is what
  this work refuses elsewhere.
- **It recovers nothing retroactively.** Events already written off will never
  have their row: fabricating an audit row dated to a fact nobody observed is the
  inverse of what this defends. The probe is the **next** occurrence.
- **It makes the fact legible, not watched.** The instruments are the grep and the
  query above, and **their silence proves nothing until somebody runs them**.
- **It does not size the population** — see S2, which is the measurement and its
  halt.

## 10. Out of scope, deliberately

- **Widening `DISPATCHABLE_REPOS` to `claude-pilot-py`** — a 2026-08-29 operator
  decision, not a predicate defect. A ticket that wants to reopen it must weigh
  it, not route around it with a surface.
- **Auto-adoption** — explicitly deferred by AC3, "until we have enough n". The
  precondition is S2's count.
- **Verification cases 1 (Dependabot half) and 2** — closed by mika#1729 and
  `ci_success_handler`. Reopening them would ship a second path for a need
  already served.
- **The `self-dev-webhook-qa` prompt** — its task-less path is already handled,
  and its remedy has nothing to do with this one.
- **An aggregated notification** (a daily digest rather than a line per event) —
  more comfortable if the volume is high, but the shape is decided **on** S2's
  measurement, not before it.
