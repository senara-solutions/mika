# Plan — mika#1823: retry `_arch_ask` once on UNPARSED disposition + route callback retry to `run_claude_pilot_groom`

## Problem

mika-arch (kimi-k2.5) occasionally emits a first-pass plan review without the required
`Disposition: READY|ITERATE|ESCALATE` suffix line. In `_iterate_groom_loop`
(`skills/bundled/_shared/dispatch-lib.sh`), `_parse_disposition` then returns empty, the
`*)` case fires with a WARN, and the loop `return 1`s immediately (PIPELINE FAILURE). The
`ready` label was already retracted at webhook step 1 by design (mika#841/#907), so the
ticket is stuck until a manual operator re-kick. Second occurrence on mika#1664 within a
single re-kick round (2026-07-23) confirms this is a recurrence class, not a one-off model
glitch.

Two defensive layers are absent or broken:

- **Layer (d)** — `_iterate_groom_loop` has **no retry** on UNPARSED first-pass
  disposition. `return 1` is immediate at `dispatch-lib.sh:2167-2170`.
- **Layer (e)** — the callback-side pipeline-failure retry
  (`skills/bundled/self-dev-callback/system_prompt.md:103`) invokes `run_claude_pilot`
  (dev-pilot), never `run_claude_pilot_groom` (dev-groom). When a **groom** callback routes
  to "On pipeline failure" (system_prompt.md:31 explicitly funnels it there), the retry
  fires the wrong tool and is rejected by the grooming-marker dispatch gate.

This ticket implements the two-part structural fix. Prompt-level "MUST" hardening (layer a)
and tier-2 fuzzy-parser widening are explicitly **out of scope** (`feedback_prompt_enforcement_fragile`
— prompt enforcement fails at loop-substrate; the structural retry is the correct layer).

## Committed position

**Two independent, additive fixes — both ship in this PR.** Part A is a bounded (single)
retry inside the shell state machine; Part B is a routing correction in a prompt-only skill
handler. Neither changes Rust engine code, DB schema, or the architect prompt.

### Key design choices (locked)

1. **Part A retry delivers the corrective prompt through `_arch_ask`'s existing stdin
   channel, not a new argument.** `_arch_ask <skill> <plan_path> [session_id]` pipes the
   file at `plan_path` to `mika ask -`. The ticket sketch's `_arch_ask "$issue_number"
   "$retry_prompt" ...` signature does **not** match the real function. The retry writes the
   corrective message to a tempfile and calls
   `_arch_ask "mika-arch-groom-ticket" "$corrective_file" "$session_id"`. `_arch_ask` is left
   unchanged.

2. **Session continuity is the mechanism.** The retry passes the `session_id` captured from
   the first-pass response, so mika-arch sees its own prior turn plus the corrective nudge and
   completes it idempotently (no duplicate review work). `session_id` is guaranteed non-empty
   at the retry site — the pre-existing `[ -n "$content1" ] && [ -n "$session_id" ]` guard
   (`dispatch-lib.sh:2075-2078`) already `return 1`s before the disposition parse if it is
   missing.

3. **Bounded to exactly one retry.** After a second UNPARSED result, the loop takes the
   existing `return 1` path — no loop, no exponential backoff (the founding failure is a
   missing-line glitch, not a transient network fault; a second immediate retry is the right
   envelope, matching AC2).

4. **The retry re-parses through the same `_parse_disposition`** (tiers 1a/1b/2 intact). A
   recovered `READY`/`ITERATE`/`ESCALATE` re-enters the existing `case "$disposition"`
   dispatch unchanged — the retry only re-populates `content1`/`disposition` before the
   `case`; every downstream branch (READY→second-pass, ITERATE→revise, ESCALATE→escalate) is
   untouched.

5. **Part B keys routing on the callback's already-established dispatch class.** The callback
   entry point already performs CALLBACK TYPE DETECTION (`system_prompt.md:7-12`) reading the
   `label` field, and a dedicated GROOM CALLBACK HANDLER (`:14-32`) routes groom callbacks —
   including their `PIPELINE FAILURE:` case — into the shared "On pipeline failure" handler.
   Part B makes that shared handler branch on dispatch class: **groom → `run_claude_pilot_groom`,
   otherwise → `run_claude_pilot`** (existing behavior). Detection uses `dispatch_class == "groom"`
   from `check_task(task_id)` (surfaced since schema v34 / mika#1001, rendered by
   `get_task.rs:61`) with the `long_running:run_claude_pilot_groom` label as the corroborating
   signal already read at entry. The existing 2-retry envelope and escalation threshold are
   preserved verbatim.

6. **Observability: an INFO log on successful retry** (`iterate_groom_loop: disposition
   recovered on retry`) plus a distinct WARN naming mika#1823 on second-UNPARSED, so
   post-deploy firing rate is greppable (AC4).

## Scope

### In scope for this PR

- **Part A** — `skills/bundled/_shared/dispatch-lib.sh`: insert a single-retry block between
  the first-pass disposition parse (`:2079`) and the `case "$disposition"` dispatch (`:2084`).
- **Part A tests** — `skills/bundled/_shared/tests/test_iterate_groom_retry.sh`: source
  dispatch-lib, override collaborators (`_arch_ask`, `_find_issue_plan`, `_parse_verdict`,
  `_write_canonical_callout`, `_cleanup_iterate_findings`, `_trail_append`, `_escalate_groom`),
  and assert AC1 (recovery → return 0) and AC2 (persistent UNPARSED → return 1). Registered in
  the test runner alongside `test_parse_disposition.sh`.
- **Part B** — `skills/bundled/self-dev-callback/system_prompt.md`: amend the "On pipeline
  failure" handler (step 4, `:103`) to branch the retry tool on dispatch class.

### Deferred / out of scope

- Prompt-level "MUST" hardening of `mika-arch-groom-ticket/system_prompt.md` (layer a) —
  `feedback_prompt_enforcement_fragile`.
- Extending engine guard #8 (`required_suffix_line_retry_done`) to 2 retries (layer b) —
  adds latency for all skills; deferred pending Part-A absorption data.
- A Sonnet-4.6 fallback model for mika-arch — separate ticket (touches calibration + baseline).
- Widening the tier-2 fuzzy parser — introduces `proceed`/`revise` prose false positives.
- Broader callback-handler rework beyond the pipeline-failure tool-routing correction.

## Acceptance criteria

- **AC1** — `_iterate_groom_loop` retries `_arch_ask` once with a corrective prompt on
  UNPARSED first-pass disposition. Verified via unit-style test: mock `_arch_ask` to return
  content without `Disposition:` on the first call and `Disposition: READY` on the second;
  assert the loop returns 0.
- **AC2** — After a second UNPARSED call, `_iterate_groom_loop` still returns 1 (bounded
  retry, no infinite loop). Verified via a mock that returns UNPARSED content on both calls.
- **AC3** — the `self-dev-callback` "On pipeline failure" handler routes to
  `run_claude_pilot_groom` when the failed dispatch was a grooming task
  (`dispatch_class = 'groom'`), preserving the existing 2-retry envelope; the dev-pilot path
  continues to use `run_claude_pilot`.
- **AC4** — an INFO log (`iterate_groom_loop: disposition recovered on retry`) is emitted on
  successful retry, and a mika#1823-tagged WARN on second-UNPARSED, for post-deploy
  firing-rate verification.
- **AC5** — Post-deploy: re-kick mika#1664 (or a fresh issue that reproduces the failure
  mode); confirm auto-groom completes without PIPELINE FAILURE and the INFO retry log appears
  (proves the fix is live, not merely deployed). Tracked as a post-merge operator step.

## Definition of Done

- Part A retry block inserted; `bash -n skills/bundled/_shared/dispatch-lib.sh` clean;
  `shellcheck skills/bundled/_shared/dispatch-lib.sh` shows no new findings.
- `test_iterate_groom_retry.sh` passes (AC1 + AC2), runnable standalone via
  `bash skills/bundled/_shared/tests/test_iterate_groom_retry.sh` and wired into the
  `_shared/tests` runner.
- Existing `test_parse_disposition.sh` and `test_find_issue_plan.sh` still pass (no regression
  from sourcing changes).
- Part B handler edit present; the groom-vs-pilot tool branch is unambiguous and the 2-retry
  escalation envelope is unchanged.
- `make verify-bundled-skills` passes (dispatch-lib is `_shared/`; both edited skills remain
  structurally valid).
- AC5 recorded in the PR body as an explicit post-merge operator step (it cannot run in CI).

## Files touched

| File | Change |
|------|--------|
| `skills/bundled/_shared/dispatch-lib.sh` | Part A — single-retry block in `_iterate_groom_loop` (`~:2079–2083`), corrective-prompt tempfile via `_arch_ask` with `session_id`, INFO/WARN logs. |
| `skills/bundled/_shared/tests/test_iterate_groom_retry.sh` | New — AC1/AC2 unit-style test with collaborator overrides. |
| `skills/bundled/self-dev-callback/system_prompt.md` | Part B — "On pipeline failure" step 4 branches retry tool on `dispatch_class`. |

## Implementation notes

### Part A — retry block shape

Inserted immediately after `disposition=$(printf '%s' "$content1" | _parse_disposition)`
(`dispatch-lib.sh:2079`) and its `_trail_append` (`:2082`), before `case "$disposition" in`:

```bash
if [[ "$disposition" != "READY" && "$disposition" != "ITERATE" && "$disposition" != "ESCALATE" ]]; then
    echo "iterate_groom_loop: first-pass disposition UNPARSED — retrying _arch_ask once (mika#1823)" >&2
    local corrective_file; corrective_file="$(mktemp "${TMPDIR:-/tmp}/arch-retry-XXXXXX.md")"
    {
        printf 'Your previous response was missing the required `Disposition:` suffix line.\n'
        printf 'Re-emit your plan review with the mandatory final non-empty line exactly as:\n'
        printf '`Disposition: READY` (or `Disposition: ITERATE` / `Disposition: ESCALATE`).\n'
    } > "$corrective_file"
    local resp_retry; resp_retry=$(_arch_ask "mika-arch-groom-ticket" "$corrective_file" "$session_id" 2>/dev/null)
    local _arch_rc=$?
    rm -f "$corrective_file"
    if [ "$_arch_rc" -eq 0 ]; then
        local content_retry; content_retry=$(printf '%s' "$resp_retry" | jq -r '.content // empty' 2>/dev/null)
        if [ -n "$content_retry" ]; then
            content1="$content_retry"
            disposition=$(printf '%s' "$content1" | _parse_disposition)
        fi
    fi
    if [[ "$disposition" != "READY" && "$disposition" != "ITERATE" && "$disposition" != "ESCALATE" ]]; then
        echo "WARN: iterate_groom_loop: first-pass disposition UNPARSED after retry (mika#1823)" >&2
        return 1
    fi
    echo "INFO: iterate_groom_loop: disposition recovered on retry (mika#1823)" >&2
fi
```

Notes:
- `session_id` is preserved (session continuity, choice 2). The retry reuses the same
  `mika-arch-groom-ticket` skill so the trail/verdict semantics downstream are unchanged.
- The pre-existing `*)` WARN at `:2167-2170` is retained as the terminal safety net for any
  disposition that is still unparsed after the retry path (defense-in-depth; the retry block's
  own `return 1` is the primary exit).
- No change to `_arch_ask`, `_parse_disposition`, or the `case` branches.

### Part A — test strategy (AC1/AC2)

dispatch-lib.sh is safe to `source` (documented "no top-level imperative code" audit in
`test_parse_disposition.sh:9-11`). The test sources it, then overrides the collaborators the
loop calls, using a call-count file to make `_arch_ask` return UNPARSED-then-READY (AC1) or
UNPARSED-twice (AC2). It stubs `_find_issue_plan` to a temp plan, `_parse_verdict`→`GROOMED`,
and no-ops `_write_canonical_callout`/`_cleanup_iterate_findings`/`_trail_append`/`_escalate_groom`,
sets `WORKTREE_DIR`/`ISSUE_NUM`/`REPO`, then asserts the loop's return code. This mirrors the
existing `_shared/tests` isolation pattern — no mika CLI, git, or network.

### Part B — handler edit shape

The "On pipeline failure" handler step (`system_prompt.md:103`, step 4) changes its retry
call from an unconditional `run_claude_pilot` to a dispatch-class branch:

> 4. Retries remain: notify "Pipeline produced no commits for {repo}#{issue_number} —
>    retrying ({n}/2)." `update_task_status` with the same `in_progress` and
>    `metadata: {"pipeline_retry_count": <current + 1>}`. Verify via `check_task`.
>    **Determine the retry tool by dispatch class:** if `check_task(task_id)` shows
>    `dispatch_class == "groom"` (equivalently, the entry-point label was
>    `long_running:run_claude_pilot_groom`), call **`run_claude_pilot_groom`** with the same
>    `repo#number` and `task_id`; otherwise call `run_claude_pilot` (existing dev-pilot
>    behavior). If the call returns `{"status": "deferred", "deferred": true}`, the retry is
>    auto-enqueued — do NOT retry again. Step 6 with `in_progress`, note "pipeline retry
>    deferred — engine will auto-dispatch when slot is free."

The escalation branch (step 3, `pipeline_retry_count >= 2`) is unchanged.

## Verification

- `bash -n skills/bundled/_shared/dispatch-lib.sh` — syntax clean.
- `shellcheck skills/bundled/_shared/dispatch-lib.sh` — no new findings.
- `bash skills/bundled/_shared/tests/test_iterate_groom_retry.sh` — AC1 + AC2 pass.
- `bash skills/bundled/_shared/tests/test_parse_disposition.sh` — no regression.
- `make verify-bundled-skills` — both edited skills remain structurally valid.
- Manual read-through of the Part B handler confirms the groom/pilot tool branch and the
  intact 2-retry envelope (AC3).
- **Post-merge (AC5, operator):** re-kick mika#1664 or a fresh reproducing issue; grep
  `server.log` for `iterate_groom_loop: disposition recovered on retry` and confirm the groom
  reaches `(GROOMED)` without PIPELINE FAILURE.

## References

- mika#1823 (this ticket); mika#1664 (founding incident, 2026-07-23)
- `docs/solutions/2026-05-21-groom-post-flight-recovery-without-architect-verdict.md:80`
  (pre-existing "dev-groom pass-2 retry-or-escalate" followup, never implemented)
- `feedback_prompt_enforcement_fragile` (why layer (a) alone is insufficient)
- `feedback_n_equals_2_is_the_signal` (recurrence-class justification)
- mika#864 (required-suffix-line guard — layer b); mika#1272 (tiered parser — layer c)
- mika#1001 / schema v34 (`dispatch_class` — enables Part B routing)
- mika#1421 v3 (tier-1b Verdict:GROOMED session-carry-over — related tier work)
- mika#1271 (dev-groom contract refactor — owner of `_iterate_groom_loop`)
