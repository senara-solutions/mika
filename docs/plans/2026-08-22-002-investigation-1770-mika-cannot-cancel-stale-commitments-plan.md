---
type: investigation
issue: mika#1770
title: Diagnose real root cause — Mika cannot cancel stale commitments despite id exposure
date: 2026-08-22
sequence: 002
priority: p1-important
scope: investigation-only (per issue "Not in scope": no code fix here)
---

# Plan — Investigation: root-cause the un-cancellable stale commitments

## Deliverable

An evidence-anchored root-cause note that:

- Names the specific candidate (A / B / C / D / E from the issue) that is
  responsible for the observed failure to cancel stale commitments.
- Cites the concrete evidence (SQL row, log line, file:line, or audit-event
  trace) that supports the naming.
- **If mika#1769 has not yet deployed**, defers the class-A verdict pending the
  post-deploy re-measurement (Step 5) and names the class-A residual to test.
- Enumerates any secondary candidates that also fire — the classes are
  mutually non-exclusive per the ticket, so a real answer may name more than
  one and rank them by contribution.
- Produces the follow-up fix ticket text (as a proposed `mika-issue` payload) —
  filed at the end of the investigation, NOT dispatched here. Filing is what
  discharges the ticket's success criterion "The specific root cause (A/B/C/D/E)
  is named with evidence."

Optional in-scope side effect (bounded, only if Step 3 confirms Mika has the
recipe): manually cancel the 7+ stale commitments via a supervised session
(cite the audit_events trace). Ticket success criterion 3 asks for the stale
list to be cleared; the un-cancellability wedge must be closed first, so this
is deferred until the root cause is named + verified.

## Non-goals (explicit)

- No code fix in this ticket (per "Not in scope" in the issue).
- No prompt edits (that's part of the follow-up fix ticket).
- No new tools or refactors.
- No dispatching the fix ticket — grooming and dispatch happen in a separate
  cycle after root cause is named.

## Baseline evidence already established at plan time

Read into the plan before dispatching, so the investigation doesn't
re-discover code shape:

- `crates/mika-agent/src/tools/search_memory.rs:242-253` — commitment output
  shape uses **label-prefix ordering** `[commitment id={id} status:{status}] {description}` on
  the LIKE-fallback path. The hybrid-search path
  (`crates/mika-agent/src/tools/search_memory.rs:96-104`) emits
  `[{source_type} id={id}] {content}` — label prefix also on line, format
  slightly different. Both put the id BEFORE the description. Post mika#1769
  code shape is IN THE TREE (verified in this worktree branched from main).
- `crates/mika-agent/src/tools/update_fact.rs:20-56` — the tool contract:
  - Requires `id` (integer), `category` ("commitment" — enum with one value),
    and `updates.status` (enum: `"completed"` | `"cancelled"` — no other
    value accepted, including `"pending"` which the DB permits).
  - Optional `evidence` (required only in reflection mode via
    `check_reflection_evidence`).
- `crates/mika-agent/src/tools/update_fact.rs:82-88` — the invalid-category
  branch returns `"Invalid category '<other>'. Currently supported: commitment"`.
  So if Mika constructs `update_fact` with category `preference` / `person` /
  `event`, she gets a hard error, not a silent no-op.
- `crates/mika-agent/src/tools/update_fact.rs:108-113` — invalid status enum
  returns `"Invalid status '<v>'. Allowed: completed, cancelled"`. So if Mika
  tries `updates.status = "pending"` or similar, she gets a hard error.
- `crates/mika-agent/src/tools/update_fact.rs:119-123` — non-existent id
  returns `"Commitment with id <id> not found."` — distinguishable from other
  error classes.

These four error shapes are the fingerprints Step 1 uses to classify the
failed `update_fact` attempts.

## Execution steps

The five ticket-body investigation steps translate to five plan phases. Each
phase names its inputs, its concrete probe, its output artifact, and its
disposition ("what does this tell us about A/B/C/D/E?").

### Phase 1 — Tool-call trace (candidate B fingerprint, candidate A residual)

**Ticket step 1.** Query `tool_calls` for Mika's `update_fact` attempts in the
past 30 days. Note failure modes, input shape, error content.

- **DB path** (per root `CLAUDE.md`): `~/.mika/data/mika.db`. The Mika agent
  container is per-customer; on the running host, Mika's DB path is
  `~/.mika/agents/mika/data/mika.db` (the well-known family agent). Both
  paths are inspected — the singleton personal-agent path is the primary.
- **Probe** — read-only SQL via `sqlite3`:

  ```sql
  SELECT
    tc.id, tc.session_id, tc.created_at, tc.input, tc.output,
    tc.is_error
  FROM tool_calls tc
  JOIN sessions s ON s.session_id = tc.session_id
  WHERE tc.tool_name = 'update_fact'
    AND tc.created_at > datetime('now', '-30 days')
  ORDER BY tc.created_at DESC;
  ```

  If the running instance is agent-scoped (`~/.mika/agents/mika/data/mika.db`),
  the `agent_id` join is implicit (single-agent DB). Otherwise filter by
  `agent_id = 'mika'`.

- **Fingerprint classification** — for each row, compare `output` to the four
  error shapes captured in the baseline block above:
  - `"Invalid category '<X>'"` → class B (Mika constructed the call for a
    category `update_fact` doesn't yet support — e.g., trying to
    "cancel" a preference or event). Or: Mika passed a mis-spelled category.
  - `"Invalid status '<X>'"` → class B or C (Mika chose a status the tool
    rejects — often "pending" from misreading the label prefix).
  - `"Commitment with id <N> not found"` → class A or B (id parse miss,
    off-by-one in extraction, or id stale from a prior turn).
  - `"'id' is required and must be a positive integer"` → class A (id extraction
    failed entirely).
  - `"evidence" ... "required"` → reflection-mode gate; not a wedge, expected.
  - No error / `is_error = 0` → the call succeeded; the wedge is downstream
    (Mika thought it worked but doesn't remember, OR the commitment was
    re-indexed with the wrong status). Cross-reference the `commitments` table
    at Phase 2 to confirm.
- **Also** collect the `input` field for each row and inspect the shape of the
  `id` value Mika constructed — is it an integer? A string? A number that
  doesn't correspond to any commitment id? The shape of the wrong-id is a
  candidate-A vs candidate-B discriminator.
- **Output artifact** —
  `/tmp/1770-investigation/phase1-update-fact-trace.md` — one row per attempt
  with class-tag and 1-line reasoning. Also a summary count by class.

### Phase 2 — Audit-event guard-firing trace (candidate D)

**Ticket step 2.** Query `audit_events` for guard firings correlated with the
Phase-1 turns.

- **Probe** — for each session_id from Phase 1, load the audit events in the
  same session, and the `messages` in the same session, both ordered by
  timestamp:

  ```sql
  SELECT ae.session_id, ae.tool_name, ae.target_key, ae.before_value,
         ae.after_value, ae.reasoning, ae.created_at
  FROM audit_events ae
  WHERE ae.session_id IN ({ids from Phase 1})
  ORDER BY ae.created_at ASC;

  SELECT id, role, content, created_at
  FROM messages
  WHERE session_id IN ({ids from Phase 1})
  ORDER BY id ASC;
  ```

- **Guard-fire fingerprints** to look for in the messages (per `CLAUDE.md`
  agent-loop § Post-Conditions and the `guard_correlation_id` telemetry, and
  in the mika-spirit log for `guard.*` structured events):
  - `guard.callback_state_claim` — unlikely (this is conversation not
    callback).
  - `guard.assert_grounded` — could fire if Mika claims the commitment was
    cancelled without a `run_gh`/`check_task` call; but commitments are Layer 2
    facts, not GitHub resources, so this is unlikely to trigger.
  - `guard.fabricated_action_claim` — could fire if Mika claims she cancelled
    without any tool call in the turn (matches the "she says she cancelled but
    the commitment stays pending" symptom).
  - Assertion patterns of "core memory says X" / "required-tools" — check for
    any skill-scoped required-tools gate on Mika's identity that could reject
    `update_fact` (unlikely — `update_fact` is a default tool, not
    skill-scoped).
- **Also** search the `mika-spirit` log for the same session_ids on `agent_id
  = 'mika'` (or the appropriate agent), looking for `guard.` events. Per the
  `CLAUDE.md` § Guard Fabrication Telemetry section, these are structured JSON
  events on `target: "mika::otel"`. Log path: `MIKA_SPIRIT_LOG_FILE` (per-host
  env) or the per-agent CLI log `~/.mika/agents/mika/logs/mika.log.<date>`.

  ```bash
  grep 'guard\.' "$MIKA_SPIRIT_LOG_FILE" | \
    jq --arg agent mika 'select(.agent_id == $agent and (.event | startswith("guard.")))'
  ```

- **Output artifact** —
  `/tmp/1770-investigation/phase2-audit-events-and-guards.md` — one row per
  guard firing (if any), one entry per session with the message-timeline
  aligned against tool-call and audit-event timeline. Explicit "no guard fired"
  conclusion if that's what the evidence shows (which would rule out D).

### Phase 3 — Recipe check (candidate C)

**Ticket step 3.** Read Mika's `identity.toml`, soul, and core_memory. Check
for the cancellation-recipe (search → extract id → update_fact).

- **Probe** —
  - `~/.mika/agents/mika/identity.toml` — read verbatim, look for a `[skills]`
    block, an `[operator]` block, any commitment/hygiene recipe.
  - `~/.mika/agents/mika/soul.md` — read verbatim, look for cancellation
    recipe.
  - `~/.mika/agents/mika/core_memory/*.md` — read the five section files
    (`user_summary.md`, `self_model.md`, `current_priorities.md`,
    `key_people.md`, `workflows.md`). Look specifically in `workflows.md` and
    `self_model.md` for the recipe.
- **What "having the recipe" looks like** — a passage that says (in prose or
  code-block form) something equivalent to: "To cancel a commitment: (1) call
  `search_memory(category='commitment', query='...')` (2) read the id from
  the `[commitment id=N status:...]` label prefix (3) call
  `update_fact(id=N, category='commitment', updates={status: 'cancelled'})`."
  The exact prose doesn't matter; the recipe must be present.
- **What "missing the recipe" looks like** — no such passage exists in any of
  the above files; core_memory mentions commitments but not how to update
  them.
- **Output artifact** —
  `/tmp/1770-investigation/phase3-recipe-check.md` — the verdict (recipe
  present / partial / missing), with a citation (file:line + excerpt) for each
  passage found (or the empty-set for missing).

### Phase 4 — Tool-schema re-read (candidate B, confirmation)

**Ticket step 4.** Read `crates/mika-agent/src/tools/update_fact.rs` — confirm
the schema and category-value handling.

- Already done at plan time (see baseline block). Re-verify against the
  worktree's checked-out code — the plan-time findings must be stable in the
  worktree branch. This phase is a **guard against code drift** between plan
  time and dispatch time (unlikely for update_fact since it's stable, but the
  discipline is mandatory).
- **Also** re-verify the **actual tool-array snapshot for Mika's container**.
  Per `CLAUDE.md` § Skills System / § Identity-driven tool denylist: Mika's
  identity.toml may declare `[tools] disabled = [...]` which could remove
  `update_fact` from her presented tool array. Query the DB or check the
  identity file:

  ```bash
  cat ~/.mika/agents/mika/identity.toml | grep -A5 '\[tools\]'
  cat ~/.mika/agents/mika/identity.toml | grep -A20 '\[skills\]'
  ```

  If `[skills].allowlist` is present and does NOT include a skill that
  registers `update_fact`, or `[tools].disabled` includes `update_fact`, that
  IS the wedge (class B, tool-not-in-array).
- **Output artifact** —
  `/tmp/1770-investigation/phase4-tool-schema-and-visibility.md` — the tool
  contract summary + Mika's `[tools]`/`[skills]` config + whether
  `update_fact` is present in Mika's turn-start enabled-tool set.

### Phase 5 — Post-mika#1769 re-measurement (candidate A verification)

**Ticket step 5.** After mika#1769 lands and deploys, re-check the rate of
failed cancellation attempts.

- **Guard condition**: first, verify mika#1769 is in the running binary.
  - `mika --version` (or `mika-spirit --version`) — capture SHA.
  - `git -C ~/dev/mika log --oneline | grep -i '1769'` on the host repo to see
    if mika#1769's merge SHA is in main and reachable from the deployed
    version. If mika#1769 has NOT merged, this phase is BLOCKED and the plan
    surfaces that as an explicit outcome (root-cause naming must defer class-A
    dismissal pending re-measurement).
- **Re-measurement** — replay the Phase-1 query, but restrict `created_at` to
  post-deploy timestamps:

  ```sql
  SELECT COUNT(*) FILTER (WHERE is_error = 1) AS failed,
         COUNT(*) FILTER (WHERE is_error = 0) AS succeeded,
         COUNT(*) AS total
  FROM tool_calls
  WHERE tool_name = 'update_fact'
    AND created_at > '<mika#1769-deploy-timestamp>';
  ```

  Ratio of failed / total is the post-deploy failure rate. Compare against the
  pre-deploy ratio (from Phase 1).
- **Class-A verdict** —
  - If failure rate → ~0: class A is confirmed as the primary wedge, and
    mika#1769 (once deployed) closes it. The 7+ stale commitments Mika can now
    cancel on retry — Vincent should prompt her, or a supervised session
    executes the cancels.
  - If failure rate stays roughly the same, class A is not the sole
    cause — the residual is class B/C/D/E, and Phase 2/3/4 evidence names the
    surviving cause(s).
  - **NEW commitment-failure attempts required.** If Mika has not attempted
    any new `update_fact` calls since mika#1769 deployed, the re-measurement
    is inconclusive — a supervised prompt is needed to elicit an attempt. That
    prompt is included in the deliverable checklist.
- **Output artifact** —
  `/tmp/1770-investigation/phase5-post-1769-remeasurement.md` — the two
  ratios + the class-A verdict + (if class A is confirmed) the supervised
  cancel-attempt trace for the 7+ stale commitments.

## Root-cause note (final deliverable)

`/tmp/1770-investigation/1770-root-cause.md` — assembled from the five
phase artifacts. Structure:

- **Class-by-class verdict.** For each of A/B/C/D/E, state "confirmed" /
  "ruled out" / "residual, contributes ~N%" / "blocked pending mika#1769
  deploy", with a one-line evidence pointer to the phase artifact.
- **Primary cause named.** The single most-load-bearing class, with the
  file:line / SQL row / log line that grounds it.
- **Follow-up fix ticket.** A proposed ticket payload in the shape of a
  `mika-issue` invocation (title + body + labels). Filed after root cause
  is named — this ticket does NOT dispatch it.
- **Stale-commitment cleanup outcome.** Whether the 7+ existing stale
  commitments were cleared (if class A confirmed and mika#1769 deployed) or
  still pending (with the specific blocker named).

## Acceptance criteria

The ticket does not carry a formal `## Acceptance Criteria` block, so these
are derived from the ticket's `## Success criteria` section (per the AC-vs-plan
reconciliation discipline — no rename, no reshape; the plan mirrors what the
issue says):

**AC1** — "The specific root cause (A/B/C/D/E) is named with evidence." Plan
tie-back: `1770-root-cause.md` § "Primary cause named" contains a class label
(A/B/C/D/E) AND a file:line / SQL row / log line citation. The five phase
artifacts each contribute evidence.

**AC2** — "Mika can successfully cancel a stale commitment in a live session,
verified via `audit_events` + tool_calls trace." Plan tie-back: after root
cause is named AND the follow-up fix ticket has resolved, a supervised session
elicits a cancel attempt. Phase 5 output artifact carries the successful
`update_fact` row (is_error=0) and the `audit_events` row confirming the
before_value → after_value transition. **Caveat**: if the root cause is class
B / C / D (not A), this criterion cannot be discharged in the investigation
ticket alone — it requires the fix ticket to land first. The plan explicitly
routes AC2 discharge to the fix ticket in that case, and the investigation
ticket closes on AC1 + a filed fix ticket that carries AC2 forward.

**AC3** — "The 7+ existing stale commitments are cleared." Plan tie-back:
same as AC2 — either discharged inline (class A + mika#1769 deployed) or
routed to the fix ticket. Neither the investigation ticket nor the plan
commits to clearing the commitments if the fix isn't yet available; the
success criterion is honestly gated on the fix path.

## Not in scope (verbatim from issue)

Grooming/dispatching a code fix here. The fix ticket will be filed after root
cause is named. **The plan honors this.** The Phase 5 exit surfaces the fix
ticket text as a proposed `mika-issue` payload; filing it is a discrete
follow-up action, not part of this ticket's execution.

## Sequencing and blockers

- Phases 1–4 are **independent** (different SQL queries, different files) and
  can run in parallel by a single agent.
- Phase 5 **blocks on mika#1769 deploy status** (checked as a Phase 5 guard).
  If undeployed, Phase 5 outputs "BLOCKED: mika#1769 not deployed", and the
  root-cause note explicitly defers class-A dismissal.
- The root-cause note **blocks on Phases 1–5** — it's the assembly of their
  outputs.
- The fix ticket is drafted from the root-cause note; filing is deferred to
  a separate operator action (see § "Not in scope").

## Discipline notes

- **Read-only investigation.** No mutations to `~/.mika/agents/mika/`, no
  writes to the DB, no edits to Mika's identity / soul / core_memory. The
  supervised cancel attempt in Phase 5 (if class A + mika#1769 deployed) IS a
  mutation — but it's a mutation Mika performs herself via her own tool call,
  not one the investigator performs on her behalf.
- **Evidence discipline.** Every classification decision must cite the concrete
  artifact (SQL row, log line, file:line). "Looks like class B" without a
  citation is not a verdict.
- **Do not touch mika#1743.** That ticket is closed not-planned; the
  invalidated-premise history there is context, not a target.
- **Do not file a preemptive fix ticket.** The fix ticket is filed AFTER the
  root cause is named, not before. Multiple candidate fixes are ranked in the
  root-cause note; only the one that matches the named cause becomes the fix
  ticket.
