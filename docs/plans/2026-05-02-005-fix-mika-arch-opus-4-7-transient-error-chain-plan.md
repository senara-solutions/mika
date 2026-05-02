---
title: "fix(mika-arch): mitigate Opus 4.7 transient-error chain via skill-LLM relocation + retry-budget tightening"
type: fix
status: active
date: 2026-05-02
---

# fix(mika-arch): mitigate Opus 4.7 transient-error chain via skill-LLM relocation + retry-budget tightening

## Overview

mika#939 documents three orthogonal failure modes observed during mika#938 grooming on 2026-05-02. This plan ships **two** of them (the addressable ones) and explicitly defers the third to a separate decision:

1. **Move `mika-arch-groom-ticket` skill from Opus 4.7 to Sonnet 4.6.** Eliminates today's failure mode (Opus transient-error chain) and matches `mika-arch-second-review` (already on Sonnet 4.6 per agent config comment). Smallest blast radius, highest confidence.
2. **Tighten the LLM retry budget at the transport layer.** Current behavior in `crates/mika-common/src/claude.rs:13` is `MAX_RETRIES = 3` with exponential backoff (500ms, 1s, 2s — totaling ~3.5s of *backoff*) but each retry waits for the per-call request timeout to expire (~2 minutes observed in incident). Effective worst case: 4 attempts × 2 min = 8 minutes — exceeds the agent deadline. Reduce to fail-fast when the remaining agent deadline is too short to fit another retry.
3. **(Deferred to separate ticket)** Sonnet 4.6 persistence-meta hallucination ("No new facts warrant persistence"). Requires deeper investigation; root cause hypothesis is unconfirmed; fix surface unclear.

## Problem Frame

Verbatim runtime log (mika#939 issue body):
```
16:40:07 INFO  using per-skill LLM override
16:40:07 INFO  llm_call started
16:42:07 WARN  transient Claude API error
16:42:07 WARN  retrying Claude API call
16:44:07 WARN  transient Claude API error
16:44:07 WARN  retrying Claude API call
16:46:08 WARN  transient Claude API error
16:46:08 WARN  retrying Claude API call
16:48:06 INFO  llm_call completed
16:48:06 WARN  agent responded without calling required tools — re-prompting
16:48:06 WARN  agent deadline exceeded — exiting loop gracefully
```

Three transient errors over 6 minutes. Each attempt waited for the API request timeout (~2 min) to expire before the *short* exponential backoff fired. Total: 8 minutes; agent deadline (configurable, see `crates/mika-agent/src/agent.rs`) was exceeded; agent emitted the user-visible fallback *"I'm sorry, that took too long. Let me try a simpler approach next time."*

This is not a model problem — Sonnet 4.6 (the agent's default per `~/.mika/agents/mika-arch/config.toml`) handled the same prompt content cleanly when both skills were disabled. The problem is the per-skill LLM override + the retry-budget shape:
- Override routes groom-ticket to Opus 4.7
- Opus today returned transient errors (Anthropic-side fluctuation; severity unknown for routine recurrence)
- Retry chain consumed the agent deadline budget without succeeding

Per `feedback_qa_provider_perf.md`: "DeepSeek best for qa-review; Claude hallucinated, MiniMax too deferential." Claude (Opus included) has hallucinated on review-style work before. The second-review skill is already on Sonnet 4.6 per the same config; groom-ticket is the inconsistent slot.

## Requirements Trace

- **R1.** `mika-arch-groom-ticket` skill routes to Sonnet 4.6 (matching `mika-arch-second-review`).
- **R2.** When the Anthropic API returns transient errors and the remaining agent deadline cannot fit another retry, the LLM transport layer aborts the retry chain immediately rather than consuming all `MAX_RETRIES`.
- **R3.** Existing retry-on-transient behavior is preserved when the remaining deadline is sufficient.
- **R4.** No change to `mika-arch-second-review` (already on Sonnet 4.6).
- **R5.** No change to other agents' Opus routing (mika-arch is the sole subject of this fix).
- **R6.** After fix: mika-arch grooming completes a two-pass review within 60 seconds per pass on a canonical brief, with both `mika-arch-groom-ticket` and `mika-arch-second-review` skills ENABLED.

## Scope Boundaries

- Only mika-arch's skill-LLM routing (`mika/skills/bundled/mika-arch-groom-ticket/skill.toml` for the per-skill `[llm:]` annotation, if it lives there) and the retry transport in `crates/mika-common/src/claude.rs` are modified.
- Other skills' LLM annotations remain unchanged.
- Other agents' configs (`~/.mika/agents/<other>/config.toml`) remain unchanged.
- The agent deadline value itself (the outer budget) is NOT modified.
- Anthropic-side reliability investigation is out of scope (filing under a separate ticket if today's incident recurs).

### Deferred to Separate Tasks

- **Sonnet 4.6 persistence-meta hallucination** ("No new facts warrant persistence"). Surfaced on two turns across two sessions. Root cause unconfirmed; could be (a) skill envelope's system prompt vocabulary, (b) Sonnet training conditioning, (c) interaction with `kimi-k2.5` orchestration shell context. Fix surface unclear. **Filing ticket title:** `fix(mika-arch): Sonnet 4.6 emits persistence-meta hallucination on review prompts (mika#939 follow-up)`. Filed before THIS PR merges.
- **Anthropic Opus 4.7 reliability monitoring.** Today's transient-error chain may indicate provider-side incident OR persistent issue. If after R1's fix any agent still routes to Opus 4.7 and shows transient-error frequency > 1% over 7 days, file investigation ticket.
- **Skill `[llm:]` annotation discoverability.** Today's debug required reading `mika skills list` output. A `mika skills show <skill>` showing the LLM annotation would help. File separately.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-common/src/claude.rs:13` — `const MAX_RETRIES: u32 = 3;` (retry budget constant)
- `crates/mika-common/src/claude.rs:452-498` — retry loop with exponential backoff (500ms × 2^(attempt-1))
- `crates/mika-common/src/claude.rs:478` — `warn!(attempt, error = %e, "transient Claude API error")` — the log entry surfaced in incident
- `crates/mika-common/src/claude.rs:458` — `warn!(attempt, delay_ms = delay.as_millis(), "retrying Claude API call")` — incident log
- `crates/mika-agent/src/agent.rs:859` — `"agent deadline exceeded — exiting loop gracefully"` — incident log; the deadline-exceeded fallback path
- `crates/mika-agent/src/agent.rs:432-489` — `deadline` parameter threaded through agent loop with deadline-clamp logic; this is the integration point for R2
- `mika/skills/bundled/mika-arch-groom-ticket/skill.toml` — likely location of the `[llm:]` per-skill annotation (verify in implementation; the annotation surfaces as `[llm: anthropic/claude-opus-4-7]` in `mika skills list` output)
- `mika/skills/bundled/mika-arch-second-review/skill.toml` — pattern to mirror; shows Sonnet 4.6 annotation (per `mika skills list` output: `[llm: anthropic/claude-opus-4-7]` on first-pass, `[llm: anthropic/claude-opus-4-7]` on second per current state — wait, both show Opus 4.7 in the audit output. Verify which one second-review uses currently. Config comment says Sonnet 4.6 — there may be drift between config comment and actual skill annotation. Implementation must reconcile.)

### Institutional Learnings

- `feedback_qa_provider_perf.md` — DeepSeek best for qa-review; Claude hallucinated, MiniMax too deferential. Architect-style review is QA-adjacent; same caution applies.
- `project_mika_arch_failure_modes.md` — prior catalog: criterion-replacement, deadline-timeout, contract-fabrication. This incident extends the catalog: Opus-transient-error-chain + skill-routing-amplification.
- mika-arch's Sonnet 4.6 default (per `~/.mika/agents/mika-arch/config.toml`) was chosen specifically so review-style work uses a faster, more reliable model. The Opus 4.7 override on groom-ticket was an inconsistent choice — second-review already on Sonnet validates the Sonnet path.

### External References

None — this is grounded in mika-side telemetry + log + config evidence.

## Key Technical Decisions

### Decision 1: Move `mika-arch-groom-ticket` to Sonnet 4.6

**Decision:** Edit the skill's per-call LLM annotation from `claude-opus-4-7` to `claude-sonnet-4-6`. Match `mika-arch-second-review`'s existing routing. Single-line change in `mika/skills/bundled/mika-arch-groom-ticket/skill.toml`.

**Rationale:**
- Eliminates today's failure mode (no more Opus = no more Opus transient-error chain).
- Matches the agent's primary default model (`anthropic_model = "claude-sonnet-4-6"`).
- Validated empirically: post-skill-disable retries used Sonnet 4.6 and delivered real verdicts.
- Smaller blast radius than transport-layer retry surgery.

**Rejected alternatives:**
- **Keep Opus 4.7 + add Sonnet fallback on N-second timeout.** Adds dual-model complexity. The Opus reasoning advantage isn't load-bearing for groom review (Sonnet handled it cleanly today).
- **Keep Opus 4.7, fix only the retry budget.** Doesn't address the underlying provider-side reliability today; defers the problem.

### Decision 2: Deadline-aware retry abort

**Decision:** Before each retry in `crates/mika-common/src/claude.rs:452-481`, check if the remaining time on the *outer* agent deadline (passed in via a new parameter) is sufficient to fit another `send_once` call (estimated by typical-call-duration constant + small buffer). If insufficient, abort the retry chain immediately with the last error rather than entering another `send_once` that will time out.

**Rationale:**
- Today's incident: 4 attempts × 2 min = 8 min wasted on retries that the deadline could not afford.
- Failing fast lets the agent emit a clean error message (or fall back to a different surface) BEFORE the deadline-exceeded fallback fires; the user-visible "I'm sorry, took too long" is uninformative.
- Preserves existing retry-on-transient behavior when the deadline budget is sufficient.

**Implementation note:** The estimated typical-call-duration constant should be conservative (e.g., 90s for Sonnet, 120s for Opus). If the remaining budget < this constant + small buffer, abort.

**Rejected alternatives:**
- **Reduce `MAX_RETRIES` to 1.** Simpler but loses the retry-on-transient when deadline IS sufficient. The deadline-aware approach is more nuanced.
- **Cap each individual retry at a shorter timeout.** Adds plumbing through `send_once`'s reqwest client; deadline-aware abort is cleaner.

### Decision 3: Defer the persistence-meta hallucination fix

**Decision:** Per Scope Boundaries → Deferred to Separate Tasks. The hallucination is real (verified in two sessions) but root cause is unconfirmed. Filing a follow-up ticket ahead of this PR's merge ensures the divergence is tracked.

**Rationale:**
- The hallucination is orthogonal to today's incident's primary failure mode (Opus transient-error chain). Decision 1 + 2 fix the Opus path; the Sonnet hallucination is a different surface.
- Premature surgery without root cause confirmed risks introducing a worse bug.
- Verification gate (R6) requires both skills enabled and grooming to complete cleanly — if the hallucination still surfaces post-Decision-1+2, the deferred ticket triggers.

## Open Questions

### Resolved During Planning

- **Where does the per-skill LLM annotation live?** → `mika/skills/bundled/mika-arch-groom-ticket/skill.toml` (likely; verify during /ce:work). Annotation key may be `llm` or `anthropic_model` or similar — implementer reads the file structure.
- **Do we need to preserve Opus reasoning for some cases?** → No. Architect's review work has been validated on Sonnet 4.6 today; the Opus override was a calibration artifact.
- **Should the deadline-aware abort log the abort reason?** → Yes. New log entry: `warn!(attempt, remaining_ms = remaining.as_millis(), "aborting retry chain — remaining deadline insufficient for another attempt")`. Makes future incidents diagnosable.

### Deferred to Implementation

- **Exact "typical call duration" constant for the deadline-aware abort.** Implementer derives from typical Sonnet 4.6 latency (~10-30s observed in audit; conservative buffer to 60s).
- **Whether `MAX_RETRIES` value should change.** Per Decision 2 rejected alternatives, the deadline-aware path is preferred over reducing MAX_RETRIES. Implementer may discover during /ce:work that retaining MAX_RETRIES=3 causes other issues; if so, surface as a question rather than silently changing.
- **Whether the skill annotation file is `skill.toml` or another format.** Implementer reads the actual file structure.

## Implementation Units

- [ ] **Unit 1: Move `mika-arch-groom-ticket` to Sonnet 4.6**

**Goal:** Edit the per-skill LLM annotation in `mika/skills/bundled/mika-arch-groom-ticket/skill.toml` (or equivalent skill-definition file) to route to `claude-sonnet-4-6` instead of `claude-opus-4-7`.

**Requirements:** R1, R4 (no change to second-review), R5 (no change to other agents).

**Dependencies:** None.

**Files:**
- Modify: `mika/skills/bundled/mika-arch-groom-ticket/skill.toml`

**Approach:**

1. Read the file and locate the LLM-routing annotation (key likely `llm`, `anthropic_model`, or similar).
2. Change value from `claude-opus-4-7` to `claude-sonnet-4-6`.
3. Verify `mika-arch-second-review`'s skill.toml: if it currently shows Opus 4.7 (per audit output), reconcile to Sonnet 4.6 to match config comment AND to keep both skills consistent. Audit found both skills annotated with Opus in `mika skills list`; the agent config comment says second-review is on Sonnet — investigate this drift during /ce:work and surface findings.

**Patterns to follow:**

- `mika/skills/bundled/mika-arch-second-review/skill.toml` — the existing-on-Sonnet pattern (verify actual state; reconcile drift if needed).

**Test scenarios:**

| Category | Scenario |
|---|---|
| Happy path | After edit, `mika skills --agent mika-arch list` shows `mika-arch-groom-ticket` annotated `[llm: anthropic/claude-sonnet-4-6]`. |
| Happy path | Live invocation: `mika ask --agent mika-arch --format json --verbose "<canonical 3KB brief>"` with both skills enabled completes within 60 seconds, emits valid `Disposition: ...` line. |
| Edge case | If `mika-arch-second-review`'s actual annotation is also Opus (drift from config comment), reconcile in same change — second-review must remain on Sonnet 4.6 per agent config intent. |
| Integration | `mika skills --agent mika-arch list` parses cleanly post-edit (no TOML syntax error). |

**Verification:**

- `mika skills --agent mika-arch list` shows correct LLM annotation on both skills.
- `mika skills validate` (if applicable to skill.toml schema) passes.
- Live invocation post-deploy completes a real grooming pass on a canonical brief.

- [ ] **Unit 2: Deadline-aware retry abort in claude.rs transport**

**Goal:** Modify `crates/mika-common/src/claude.rs:452-498` retry loop to abort when the remaining outer deadline cannot afford another `send_once` attempt. Preserves retry-on-transient when deadline is sufficient.

**Requirements:** R2, R3.

**Dependencies:** None (orthogonal to Unit 1; ships in same PR).

**Files:**
- Modify: `crates/mika-common/src/claude.rs` (retry loop at lines ~452-498; possibly add a new public function param or context struct)
- Modify: callers in `crates/mika-agent/src/` that thread the deadline through to the LLM call (verify the call sites — agent.rs:432-489 has deadline plumbing already)
- Test: `crates/mika-common/src/claude.rs` (`#[cfg(test)] mod tests` if it exists; otherwise add)

**Approach:**

1. Add a deadline parameter (or context struct) to the public retry-aware send function. Default to `None` for callers that don't have deadline visibility (preserves backwards compatibility).
2. Before each retry attempt, if `deadline.is_some()`, compute `remaining = deadline - now`. If `remaining < TYPICAL_CALL_DURATION + RETRY_BUFFER` (constants TBD per Decision 2), break out of the retry loop with the last error rather than entering another `send_once`.
3. Log the abort: `warn!(attempt, remaining_ms = remaining.as_millis(), "aborting retry chain — remaining deadline insufficient for another attempt")`.
4. Existing retry behavior preserved when `deadline.is_none()` or when remaining budget is sufficient.

**Patterns to follow:**

- `crates/mika-agent/src/agent.rs:432-489` — existing deadline-clamp logic. Mirror the same posture: deadline as `Instant`, conservative buffer, fast-fail on insufficient budget.

**Test scenarios:**

| Category | Scenario |
|---|---|
| Happy path | First attempt succeeds → no retry, no deadline check, identical behavior to today. |
| Happy path | First attempt fails transiently, deadline has 5+ minutes remaining → retry with backoff, identical to today. |
| Edge case | Second attempt fails transiently, deadline has only 30 seconds remaining (less than `TYPICAL_CALL_DURATION` constant) → abort with last error, log abort reason; do NOT enter third `send_once`. |
| Edge case | First attempt fails non-transiently (non-retryable error) → return immediately, no deadline check needed (existing behavior). |
| Edge case | `deadline.is_none()` (caller without deadline visibility) → fall back to today's behavior; full `MAX_RETRIES` retry chain. |
| Error path | `MAX_RETRIES` exhausted with retryable errors AND deadline still has budget → return last error (existing behavior). |
| Integration | Replay incident-shape: 3 transient errors with deadline running out → abort fires at attempt 2 or 3, total elapsed time ~3-5 minutes (not 8). |

**Verification:**

- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all` — new fixtures + existing tests pass.
- Replay-incident test passes: simulating the timing of today's incident (3 transient errors over 6 min, deadline = 8 min) shows abort fires before exhausting MAX_RETRIES.
- Backward-compatibility check: callers that don't pass deadline see no behavior change.

## System-Wide Impact

- **Interaction graph:** mika-arch is the only agent affected by Unit 1 (other agents' skill annotations unchanged). Unit 2 affects every caller of the LLM retry transport — verify by grepping callers and confirming they either pass deadline (new behavior) or pass `None` (existing behavior).
- **Error propagation:** Unit 2 changes WHEN the retry chain returns an error; the error type/shape is unchanged. Callers' error-handling unchanged.
- **State lifecycle risks:** None. Both units are config + transport changes with no persistent state mutation.
- **API surface parity:** If Unit 2 adds a parameter to a public function, downstream crates/binaries need recompilation. Use `Option<Instant>` so default-`None` preserves call-site signatures with minimal churn.
- **Integration coverage:** Live invocation post-deploy + canary on mika#931 (the dev-groom canary that started this whole chain) is the integration test. mika#931 will exercise mika-arch via /mika-ask-arch on real briefs — if the fix works, no `[denied]`, no `"I'm sorry"`, no fabrication.
- **Unchanged invariants:**
  - mika-arch's default model (Sonnet 4.6) — unchanged.
  - Other agents' models — unchanged.
  - `MAX_RETRIES = 3` value — unchanged (Decision 2 changes WHEN retries abort, not the budget value).
  - Retry backoff curve — unchanged.
  - Non-retryable error handling — unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Unit 1 breaks something (skill annotation file format isn't what we think). | /ce:work reads the actual file structure; surfaces unexpected format as an implementation question rather than guessing. |
| Unit 2's `TYPICAL_CALL_DURATION` constant too aggressive — aborts retries that would have succeeded. | Conservative tuning (60s+ buffer); ship with widely-margined value, observe in production. |
| `mika-arch-second-review` audit-output drift from config-comment intent. | Unit 1 reconciles; tests verify both skills land on Sonnet. |
| Anthropic Opus 4.7 transient errors recur; if other agents still on Opus, they hit the same incident. | Out of scope for this fix. Filed under deferred ticket "Anthropic Opus 4.7 reliability monitoring." |
| Sonnet persistence-meta hallucination still surfaces post-Decision-1+2. | Decision 3 explicitly defers; verification (R6) catches if it still occurs. |
| Plan-doc-check hook fails on PR open because the plan path isn't cited in the PR body or commit. | Manually cite the literal path `docs/plans/2026-05-02-005-fix-mika-arch-opus-4-7-transient-error-chain-plan.md` in the PR body or a commit body. |

## Documentation / Operational Notes

- **Rollout:** Skill change (Unit 1) is a `mika/skills/bundled/` edit — propagates via `make deploy` + bundled-skill re-sync per `mika#923`-related mechanism. Transport change (Unit 2) requires `cargo build --release` + binary deploy. Both ship in same PR for atomic rollout.
- **Verification timeline:** After PR merges and `make deploy` completes:
  1. `mika skills --agent mika-arch list` shows correct annotations.
  2. Direct invocation: `mika ask --agent mika-arch --format json --verbose "<test brief>"` completes within 60s.
  3. Re-fire dev-groom canary on mika#931 — should reach Phase 3 step 9 architect call cleanly. If mika#938 fix has not yet shipped, the canary may still hit the pre-classifier deny separately; isolate by direct invocation first.
- **Monitoring follow-up:** After 7 days post-deploy, query: `SELECT COUNT(*) FROM ... WHERE message LIKE '%transient Claude API error%' AND agent_id = 'mika-arch'`. If non-zero, file Anthropic-side investigation per Deferred to Separate Tasks.

## Sources & References

- **Ticket:** [mika#939](https://github.com/senara-solutions/mika/issues/939)
- **Surfacing canary:** mika#938 grooming sessions, 2026-05-02 ~16:40-16:57 UTC.
- **DB telemetry:** sessions `245a3c48-1be5-4ee8-89fd-6711c299ede6` (failed) and `c65a98c7-b2a1-4a9d-9a98-7d9910f509f1` (successful).
- **Runtime log:** `~/.mika/agents/mika-arch/logs/mika.log.2026-05-02` (the verbatim transient-error-chain quoted in Problem Frame).
- **Source files:**
  - `crates/mika-common/src/claude.rs:13` — MAX_RETRIES constant
  - `crates/mika-common/src/claude.rs:452-498` — retry loop (Unit 2 surface)
  - `crates/mika-agent/src/agent.rs:432-489` — deadline plumbing precedent
  - `crates/mika-agent/src/agent.rs:859` — deadline-exceeded fallback emission
  - `mika/skills/bundled/mika-arch-groom-ticket/skill.toml` — Unit 1 surface
  - `mika/skills/bundled/mika-arch-second-review/skill.toml` — pattern reference
  - `~/.mika/agents/mika-arch/config.toml` — agent default model
- **Related institutional knowledge:**
  - `feedback_qa_provider_perf.md` — Claude hallucinates on review work
  - `project_mika_arch_failure_modes.md` — prior failure-mode catalog
- **Predecessor blockers (resolved):** mika#935 / PR #937 (relay-deny layer 1); mika-platform#76 / PR #78 (Phase 1 step 4 interactive bug). mika#938 still pending — orthogonal to this fix; both can ship independently.
