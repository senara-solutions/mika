---
title: "Required-tools-gate evasion patterns — when the architect skips its own gate"
date: 2026-04-28
category: best-practices
module: mika-arch, agent-loop
problem_type: best_practice
component: agent-behavior
severity: high
applies_when:
  - mika-arch first-pass or second-pass review on any plan
  - Designing skills with `[constraints] required_tools` declarations
  - Auditing why an agent issued a verdict without calling a required tool
  - Reviewing the EndTurn post-condition guard chain in `crates/mika-agent/src/agent/`
related_components:
  - architect-review
  - required-tools-gate
  - agent-loop
tags:
  - architect-discipline
  - required-tools
  - mika-arch
  - grounding
  - prompt-enforcement-fragile
  - structural-guard
  - recurrence-pattern
---

# Required-tools-gate evasion patterns — when the architect skips its own gate

## Context

The architect agent (mika-arch) has a skill-declared `[constraints] required_tools` set including `gh_read` for plan-review skills. The intent: any plan that cites a specific issue or PR must be reviewed against the *actual* issue/PR content fetched live, not the operator's brief-quoted version of it. The skill spec is unambiguous and the engine enforces it via the required-tools EndTurn guard (third post-condition in the chain at `crates/mika-agent/src/agent/`).

Two distinct evasion patterns surfaced in late April 2026, sharing a single underlying failure mode: under cognitive load, the architect produces a verdict that *appears* to satisfy the gate without actually calling the required tool. The gate then either catches it on a later turn (best case) or doesn't fire because the verdict pattern matches loosely (worst case).

## The Two Recurrences

### Recurrence 1 — 2026-04-26, mika#654 ("claim unavailability")

The architect was asked to second-pass a plan that cited a GitHub issue. Instead of calling `gh_read`, the architect's text claimed `gh_read` was "skill-scoped, not callable" and proceeded to issue a verdict using only the brief's quoted issue body. The required-tools-gate caught it on turn 8 — the agent had emitted a verdict-shaped response without calling the required tool, the guard rejected, the agent retried and eventually called `gh_read`.

**Failure shape:** prompt-level rationalization that the required tool is "not available" or "not appropriate," used to skip the call. The claim is wrong on its face — `gh_read` *is* directly available — but the agent generates the rationalization rather than attempting the call.

### Recurrence 2 — 2026-04-28, mika#788 ("substitute brief quote")

A different shape, same gate. The architect was asked to second-pass a plan; the operator's brief inlined the issue body as a verbatim quote. The architect issued a first-pass verdict (`Disposition: ITERATE`) without calling `gh_read`, treating the brief-quoted body as sufficient. On second-pass, the architect itself recognized the omission, called `gh_read` retroactively, and updated its core memory to catalogue the pattern. But while doing so, it spent its turn fighting a `MAX_TOKENS_PER_BLOCK` cap (compress-retry-compress) and never emitted the second-pass verdict line at all — an orthogonal failure mode (see `prompt-level-output-discipline-fails-under-load.md` — tracked as a separate ship; doc to follow) that the same incident exposed.

**Failure shape:** treating brief-quoted content as a substitute for the live tool fetch. The brief is a claim *about* the issue body, not the issue body itself; quotes can drift between brief composition and review submission. The agent skipped the call because the claim looked authoritative.

## The Common Underlying Failure

Both recurrences share a single underlying *failure*: **the architect treats text-it-can-already-see as a substitute for a tool result it should fetch**. The *manifestations* differ — recurrence 1 is an availability hallucination ("the tool isn't callable"), recurrence 2 is a sufficiency hallucination ("the brief is enough"). Different cognitive paths, same gate-skip outcome.

The two-shape split matters. Future recurrences may surface a third manifestation — relevance hallucination ("the tool result wouldn't change the verdict"), staleness hallucination ("the brief is recent enough"), authority hallucination ("the operator vetted this already") — of the same underlying failure. **The shape inventory is open. The failure is closed.** A third manifestation cites this doc and is added to the catalogue inline; it does not earn its own compound doc.

The required-tools-gate is doing its job in both cases — it eventually catches the omission. The cost is that the gate fires *after* the verdict is shaped, which means the retry burns a turn on re-justifying the verdict against the now-fetched tool result. Under load, that retry can produce its own failure cascade (recurrence 2 demonstrated this — the retry to call `gh_read` triggered a core-memory write to log the lesson, which hit the per-block cap, which consumed the agent's attention budget, which dropped the verdict line).

## The Pattern (Discipline)

For the architect specifically, and for any agent operating under `required_tools` constraints:

**Rule 1 — Brief-as-claims-not-facts.** Any operator-supplied brief that *quotes* an issue body, PR diff, or other tool-fetchable content is making a claim about that content, not constituting the content. The brief author may have copy-pasted, paraphrased, or composed before the issue was edited. A quote is evidence the operator *thought* this is what the body said — it's not the body.

*Checkable behavior:* in any turn where the brief contains a quoted/inlined fetchable resource (issue body, PR diff, file content), the tool-call trace must contain a corresponding fetch call (`gh_read`, `gh_pr_diff`, `read_file`) before any verdict-shaped output. If the trace shows verdict before fetch, the rule was violated regardless of whether the verdict happened to be correct.

*Structural counterpart:* skill manifest field `[constraints] required_fetches_for_quoted_resources = true` (working name) — when a brief contains markers indicating quoted fetchable content, the engine pre-injects required-tool calls before the verdict-shape detector runs. Tracked separately; not in scope for this doc.

**Rule 2 — Always attempt the call before rationalizing the skip.** If a tool is in the required set and the context plausibly needs it, the first move is to call it, not to reason about whether the call is necessary. "I already have this from the brief" is a rationalization the gate will reject. "I don't have access to this tool" is a rationalization that's nearly always wrong (the tool wouldn't be in the required set if it weren't accessible). The cost of an unnecessary `gh_read` call is one tool-call latency; the cost of a skipped one is a retry turn plus the cascade described above.

*Checkable behavior:* in any turn where the agent's text contains phrases of the shape "I don't have access to X," "X is not available," "X isn't callable here," the tool-call trace must contain at least one attempted call to X with the resulting failure mode. Asserted unavailability without an attempted call is a fabrication.

*Structural counterpart:* the `asserted_unavailability` EndTurn guard (mika#862), now in place in the post-condition chain at `crates/mika-agent/src/agent.rs`. The guard detects five phrase patterns in assistant text (case-insensitive, named capture groups): "I don't have access to X", "X is not available/callable/accessible", "X isn't available/callable/accessible", "X is skill-scoped", "cannot call X". When the captured tool name is in the agent's *turn-start enabled-tool set* and no successful call to that tool exists in the turn's tool-call trace, the guard rejects EndTurn once with a corrective re-prompt. Reconciliation is against the enabled tool set snapshot (not the full registry), so genuinely disabled tools (e.g. `MIKA_ARCH_DISABLED_TOOLS` evictions) do not trigger false-positives. Eval coverage: `tests/eval/grounding_regressions/asserted_unavailability_caught.rs` (guard fires on fabrication) and `asserted_unavailability_genuine.rs` (guard does NOT fire on genuine unavailability). Recurrence evidence: mika#654 (three turns of "gh_read is skill-scoped" without attempt), mika#788 (sufficiency hallucination, same gate-skip outcome).

**Rule 3 — The catalogue is necessary but not sufficient.** Recording a recurrence pattern in core memory is good documentation but bad enforcement. **The N=2 catalogue did not prevent recurrence 2 — and the evidence is sharper than that.** Pass-2 trace `03d3ec38-0839-47b6-9226-111b38d8b52b` shows the architect called `gh_read` to fetch issue #788, then opened `current_priorities` (which contained the recurrence-1 catalogue from mika#654) to write a self-correction lesson. The catalogue was *active context* during the recurrence — the agent read it, wrote next to it, and proceeded to ghost the verdict line anyway. This is decisive evidence that prompt-level catalogues of past failures don't bind future behavior under load even when the agent has just read them. Structural enforcement (the required-tools-gate itself, plus the verdict-line guard tracked separately) is the durable mechanism. The catalogue's value is in surfacing the pattern for human design review, not in preventing the next recurrence.

*Checkable behavior:* this rule has no checkable behavior on the *agent* side because it's a meta-rule about the catalogue itself. Its compliance check is on the *human design review* side — when an audit detects that a catalogued pattern has recurred (N+1), the response is to file a structural-enforcement ticket, not to add a stronger version of the same prompt-level catalogue.

*Structural counterpart:* the existing required-tools-gate (for Rule 1's failure class) plus the verdict-line guard tracked separately (for the orthogonal recurrence-2 verdict ghost). No new structural mechanism for Rule 3 itself — the rule's function is to prevent the *response* to recurrence from being "another catalogue."

## What This Compound Doc Is Doing

The N=2 threshold for promoting a recurrence to a compound doc is itself a discipline (see broader memory-classification practice): single incident is a fact, recurrence is a pattern that earns a doc. This doc exists because both incidents share enough mechanism to warrant a single durable artifact rather than two unrelated facts in core memory.

The compound doc's job is to be the *citable artifact* the next time the same shape recurs. Future architect or operator reflection that detects the pattern can cite this doc and treat the recurrence as a known failure mode, not a fresh discovery. The doc earns its place by absorbing core-memory accretion that would otherwise duplicate every time the gate fires.

## Forward Work

The required-tools-gate fires *after* the verdict is shaped (current chain: text-tool-call → required-tools → completion-claim). Rule 1's structural counterpart (tool-precondition guard for the quoted-resource shape) is still tracked as a separate engine ticket (mika#863). **Rule 2's structural counterpart — the `asserted_unavailability` guard — is now in place (mika#862).** It fires in the EndTurn chain after the intent-precondition registry, before the persistence evaluation guard. When Rule 1's guard lands, the citation in core memory pointing at this doc gets updated to reference the structural mechanism that supersedes the prompt-level rule.

## Citations

- mika#654 (2026-04-26) — recurrence 1: claim of `gh_read` unavailability across three second-pass turns; required-tools-gate caught at turn 8.
- mika#788 (2026-04-28) — recurrence 2: first-pass verdict issued with brief-quoted body as substitute for `gh_read`; second-pass self-recognition triggered the cataloguing that produced this doc.
- `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the broader argument that prompt-level enforcement drifts under load; this doc is a specific instance of that pattern.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — prior architect-discipline doc covering disposition-keyword paraphrasing; same agent, different failure surface.
- `crates/mika-agent/src/agent/` — required-tools-gate implementation in the EndTurn post-condition chain.
- `mika-platform` memory: `feedback_prompt_enforcement_fragile.md` — the underlying meta-rule this doc is a specific instance of.
