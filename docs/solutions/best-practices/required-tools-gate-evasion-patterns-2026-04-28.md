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

## The Recurrences

### Recurrence 1 — 2026-04-26, mika#654 ("claim unavailability")

The architect was asked to second-pass a plan that cited a GitHub issue. Instead of calling `gh_read`, the architect's text claimed `gh_read` was "skill-scoped, not callable" and proceeded to issue a verdict using only the brief's quoted issue body. The required-tools-gate caught it on turn 8 — the agent had emitted a verdict-shaped response without calling the required tool, the guard rejected, the agent retried and eventually called `gh_read`.

**Failure shape:** prompt-level rationalization that the required tool is "not available" or "not appropriate," used to skip the call. The claim is wrong on its face — `gh_read` *is* directly available — but the agent generates the rationalization rather than attempting the call.

### Recurrence 2 — 2026-04-28, mika#788 ("substitute brief quote")

A different shape, same gate. The architect was asked to second-pass a plan; the operator's brief inlined the issue body as a verbatim quote. The architect issued a first-pass verdict (`Disposition: ITERATE`) without calling `gh_read`, treating the brief-quoted body as sufficient. On second-pass, the architect itself recognized the omission, called `gh_read` retroactively, and updated its core memory to catalogue the pattern. But while doing so, it spent its turn fighting a `MAX_TOKENS_PER_BLOCK` cap (compress-retry-compress) and never emitted the second-pass verdict line at all — an orthogonal failure mode (verdict-ghosting under cognitive load). This is now structurally guarded by mika#864's `required_suffix_line` EndTurn post-condition (manifest-driven `[output] required_suffix_lines` opt-in, last-3-non-empty-lines scan with single-retry, structural counterpart for trace `03d3ec38-0839-47b6-9226-111b38d8b52b`). See also the pending `prompt-level-output-discipline-fails-under-load.md` companion doc which cites this guard as its structural floor.

**Failure shape:** treating brief-quoted content as a substitute for the live tool fetch. The brief is a claim *about* the issue body, not the issue body itself; quotes can drift between brief composition and review submission. The agent skipped the call because the claim looked authoritative.

### Recurrence 3 — 2026-04-29, mika#893 ("elided-copula shape")

The architect was asked to first-pass groom a plan. The first turn emitted `gh_read not callable in CLI session` — the same asserted-unavailability failure class as recurrence 1, but with the copula `is` elided. The structural guard from mika#862 was deployed and active in this session, but its regex pattern 2 required `<tool> is not callable` with the `is` present. The elided form `<tool> not callable` slipped through. The required-tools gate caught it one post-condition later (same safety-net as recurrence 1), and `gh_read` succeeded immediately on the retry.

**Failure shape:** same underlying failure as recurrence 1 (claim unavailability without attempting the call), but a new *linguistic* manifestation — the English copula `is` is optional in the architect's phrasing. The structural guard's regex was overly specific. Closed by mika#894's regex extension: P2 now uses `(?:is )?` for optional copula and `(?:\w+ly )?` for optional adverb interposition. P4 gains the same optional copula.

**Evidence:** N=6 cumulative instances across mika#654, #863, #886, #890, #893, and earlier occurrences. See mika#894 issue body for the full evidence table.

## The Common Underlying Failure

Both recurrences share a single underlying *failure*: **the architect treats text-it-can-already-see as a substitute for a tool result it should fetch**. The *manifestations* differ — recurrence 1 is an availability hallucination ("the tool isn't callable"), recurrence 2 is a sufficiency hallucination ("the brief is enough"). Different cognitive paths, same gate-skip outcome.

The two-shape split matters. Future recurrences may surface a third manifestation — relevance hallucination ("the tool result wouldn't change the verdict"), staleness hallucination ("the brief is recent enough"), authority hallucination ("the operator vetted this already") — of the same underlying failure. **The shape inventory is open. The failure is closed.** A third manifestation cites this doc and is added to the catalogue inline; it does not earn its own compound doc.

The required-tools-gate is doing its job in both cases — it eventually catches the omission. The cost is that the gate fires *after* the verdict is shaped, which means the retry burns a turn on re-justifying the verdict against the now-fetched tool result. Under load, that retry can produce its own failure cascade (recurrence 2 demonstrated this — the retry to call `gh_read` triggered a core-memory write to log the lesson, which hit the per-block cap, which consumed the agent's attention budget, which dropped the verdict line).

## The Pattern (Discipline)

For the architect specifically, and for any agent operating under `required_tools` constraints:

**Rule 1 — Brief-as-claims-not-facts.** Any operator-supplied brief that *quotes* an issue body, PR diff, or other tool-fetchable content is making a claim about that content, not constituting the content. The brief author may have copy-pasted, paraphrased, or composed before the issue was edited. A quote is evidence the operator *thought* this is what the body said — it's not the body.

*Checkable behavior:* in any turn where the brief contains a quoted/inlined fetchable resource (issue body, PR diff, file content), the tool-call trace must contain a corresponding fetch call (`gh_read`, `gh_pr_diff`, `read_file`) before any verdict-shaped output. If the trace shows verdict before fetch, the rule was violated regardless of whether the verdict happened to be correct.

*Structural counterpart:* the `required_fetches_for_quoted_resources` pre-fetch guard (mika#863), now in place in the skills pipeline at `crates/mika-agent/src/skills/quoted_resources.rs`. The guard fires at turn-start (before the LLM generates text) when a keyword-matched skill declares `[constraints] required_fetches_for_quoted_resources = true` and the user message contains triple-backtick-fenced blocks with recognizable resource headers (issue bodies, PR diffs, file content quotes, `gh issue view`/`gh pr view`/`gh pr diff` command output). Detected resources are mapped to `gh_read` and merged into the required-tools set. The existing required-tools gate at EndTurn then enforces the augmented set — but earlier in the turn, eliminating the verdict-then-retry waste observed in mika#788. Detection is conservative (fenced content only, not prose `#NNN` references) and opt-in per skill. mika-arch's `mika-arch-groom-ticket` and `mika-arch-second-review` skills opt in. Eval coverage: `tests/eval/grounding_regressions/quoted_resource_pre_fetch.rs` (three scenarios: caught, no-op, mixed brief). Recurrence evidence closed: mika#788 trace shape (verdict issued against brief-quoted issue body without calling `gh_read`).

**Rule 2 — Always attempt the call before rationalizing the skip.** If a tool is in the required set and the context plausibly needs it, the first move is to call it, not to reason about whether the call is necessary. "I already have this from the brief" is a rationalization the gate will reject. "I don't have access to this tool" is a rationalization that's nearly always wrong (the tool wouldn't be in the required set if it weren't accessible). The cost of an unnecessary `gh_read` call is one tool-call latency; the cost of a skipped one is a retry turn plus the cascade described above.

*Checkable behavior:* in any turn where the agent's text contains phrases of the shape "I don't have access to X," "X is not available," "X isn't callable here," the tool-call trace must contain at least one attempted call to X with the resulting failure mode. Asserted unavailability without an attempted call is a fabrication.

*Structural counterpart:* the `asserted_unavailability` EndTurn guard (mika#862), now in place in the post-condition chain at `crates/mika-agent/src/agent.rs`. The guard detects five phrase patterns in assistant text (case-insensitive, named capture groups): "I don't have access to X", "X is not available/callable/accessible", "X isn't available/callable/accessible", "X is skill-scoped", "cannot call X". When the captured tool name is in the agent's *turn-start enabled-tool set* and no successful call to that tool exists in the turn's tool-call trace, the guard rejects EndTurn once with a corrective re-prompt. Reconciliation is against the enabled tool set snapshot (not the full registry), so genuinely disabled tools (e.g. `MIKA_ARCH_DISABLED_TOOLS` evictions) do not trigger false-positives. Eval coverage: `tests/eval/grounding_regressions/asserted_unavailability_caught.rs` (guard fires on fabrication) and `asserted_unavailability_genuine.rs` (guard does NOT fire on genuine unavailability). Recurrence evidence: mika#654 (three turns of "gh_read is skill-scoped" without attempt), mika#788 (sufficiency hallucination, same gate-skip outcome).

**Rule 3 — The catalogue is necessary but not sufficient.** Recording a recurrence pattern in core memory is good documentation but bad enforcement. **The N=2 catalogue did not prevent recurrence 2 — and the evidence is sharper than that.** Pass-2 trace `03d3ec38-0839-47b6-9226-111b38d8b52b` shows the architect called `gh_read` to fetch issue #788, then opened `current_priorities` (which contained the recurrence-1 catalogue from mika#654) to write a self-correction lesson. The catalogue was *active context* during the recurrence — the agent read it, wrote next to it, and proceeded to ghost the verdict line anyway. This is decisive evidence that prompt-level catalogues of past failures don't bind future behavior under load even when the agent has just read them. Structural enforcement (the required-tools-gate itself, plus the verdict-line guard tracked separately) is the durable mechanism. The catalogue's value is in surfacing the pattern for human design review, not in preventing the next recurrence.

*Checkable behavior:* this rule has no checkable behavior on the *agent* side because it's a meta-rule about the catalogue itself. Its compliance check is on the *human design review* side — when an audit detects that a catalogued pattern has recurred (N+1), the response is to file a structural-enforcement ticket, not to add a stronger version of the same prompt-level catalogue.

*Structural counterpart:* the existing required-tools-gate (for Rule 1's failure class) plus the verdict-line guard tracked separately (for the orthogonal recurrence-2 verdict ghost). No new structural mechanism for Rule 3 itself — the rule's function is to prevent the *response* to recurrence from being "another catalogue."

**Rule 4 — Pattern coverage is verbatim.** When a structural guard uses regex pattern matching, the test fixture set must include every catalogued recurrence's *verbatim* phrasing (not a normalized form). A pattern that catches "X is not Y" but misses "X not Y" is a false-floor guard — it appears to protect against the failure class but only protects against the typed-out form. Defense: any new recurrence catalogue entry MUST be added as a regex test fixture in the same PR that catalogues it.

*Checkable behavior:* when a recurrence is added to the N catalogue, the PR includes a new test case in the matching fixture file with the recurrence's verbatim phrasing. If a guard fires on the test case, the regression is closed. If the guard does not fire, the regex must be extended in the same PR.

*Canonical fixture locations:*
- `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_caught.rs` (Patterns 1, 2, 4 — canonical phrasings)
- `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_genuine.rs` (false-positive defense — genuinely disabled tool)
- `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_elided_copula.rs` (Patterns 2, 3, 4 elided/adverb-interposed shapes — added by mika#894)
- `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_extension_shapes.rs` (Patterns 6, 7, 8, 9 — descriptor absorption, antonym unavailable, modal negation — added by mika#1177)
- `crates/mika-agent/src/evidence/guards.rs` `#[cfg(test)] mod tests` block — unit tests for `detect_asserted_unavailability()` directly (registry-filter and case-insensitive coverage)

A new contributor adding a recurrence to the N catalogue should add the verbatim phrasing to whichever file's shape it matches, and add a frozen `*_pre_fix.json` fixture to `tests/eval/grounding_regressions/fixtures/` alongside.

## What This Compound Doc Is Doing

The N=2 threshold for promoting a recurrence to a compound doc is itself a discipline (see broader memory-classification practice): single incident is a fact, recurrence is a pattern that earns a doc. This doc exists because both incidents share enough mechanism to warrant a single durable artifact rather than two unrelated facts in core memory.

The compound doc's job is to be the *citable artifact* the next time the same shape recurs. Future architect or operator reflection that detects the pattern can cite this doc and treat the recurrence as a known failure mode, not a fresh discovery. The doc earns its place by absorbing core-memory accretion that would otherwise duplicate every time the gate fires.

## Forward Work

The required-tools-gate fires *after* the verdict is shaped (current chain: text-tool-call → required-tools → completion-claim). **Rule 1's structural counterpart — the `required_fetches_for_quoted_resources` pre-fetch guard — is now in place (mika#863).** It fires at turn-start in the skills pipeline, before the LLM generates text. When a keyword-matched skill opts in and the user message contains quoted fetchable resources, the corresponding fetch tools are pre-injected into the required set. **Rule 2's structural counterpart — the `asserted_unavailability` guard — is now in place (mika#862) and regex-extended (mika#894).** It fires in the EndTurn chain after the intent-precondition registry, before the persistence evaluation guard. The #894 extension closed the elided-copula and adverb-interposed regex gaps that allowed N=4 catalogued phrasings to escape the guard. **Rule 4 (verbatim-fixture discipline)** governs pattern maintenance going forward — every new recurrence adds its verbatim phrasing as a test fixture in the same PR.

**Residual coverage (post-#894 adversarial review) — now closed by mika#1177.** Three escape shapes remained after the #894 extension — surfaced by adversarial code review on the #894 PR and filed as separate follow-ups, not silently absorbed. All three are now covered:

- **(a) Descriptor-word absorption** — `the gh_read tool is not available` (P2 leftmost-match captured `tool`, not `gh_read`). **Fixed by P6** (`mika#1177`): a dedicated pattern captures the tool name *before* the descriptor noun (`tool`/`function`/`feature`/`skill`/`handler`).
- **(b) Antonym `unavailable`** — `gh_read is currently unavailable` (the single-word adjective was not in P2's `not (?:available|callable|accessible)` alternation). **Fixed by P7** (`mika#1177`): standalone `unavailable` pattern with optional copula and adverb.
- **(c) Modal / periphrastic negation** — `gh_read may not be callable`, `gh_read could not be called`, `gh_read doesn't appear to be callable`, `unable to call gh_read`. **Fixed by P8 and P9** (`mika#1177`): P8 covers modal verbs (`may/could/cannot/can't/won't/wouldn't [not] be ...`) and `doesn't appear/seem to be ...`; P9 covers the inverted form (`unable to call/invoke/use/access/reach X`).

Composition gap: the `has_successful_pr_review` early-accept path previously skipped the asserted-unavailability guard (6c) and the assert-grounded guard (6d) alongside completion-claim and action-claim — fixed by mika#1178 (guards 6c and 6d now fire regardless of `skip_remaining_guards`). With P6-P9 in place, the asserted-unavailability guard covers all catalogued escape shapes. Future recurrences will surface new linguistic forms; the shape inventory remains open per the meta-rule above.

## Citations

- mika#654 (2026-04-26) — recurrence 1: claim of `gh_read` unavailability across three second-pass turns; required-tools-gate caught at turn 8.
- mika#788 (2026-04-28) — recurrence 2: first-pass verdict issued with brief-quoted body as substitute for `gh_read`; second-pass self-recognition triggered the cataloguing that produced this doc.
- mika#893 (2026-04-29) — recurrence 3: elided-copula shape (`gh_read not callable in CLI session`); asserted-unavailability guard (#862) bypassed due to regex coverage gap; required-tools-gate safety net caught it. Closed by mika#894 regex extension.
- mika#894 (2026-05-13) — regex-extension fix: P2 optional copula + adverb, P3 adverb, P4 optional copula. Closes the elided-copula and adverb-interposed gaps. Adds Rule 4 (verbatim-fixture discipline).
- mika#1177 (2026-06-12) — regex-extension fix: P6 descriptor-word absorption, P7 antonym `unavailable`, P8 modal/periphrastic negation, P9 inverted modal `unable to`. Closes the three residual escape shapes surfaced by adversarial review on the #894 PR. Eval coverage: `asserted_unavailability_extension_shapes.rs` (6 tests: 3 caught + 3 pre-fix regression).
- mika#1178 (2026-05-17) — has_successful_pr_review skip-path composition gap. Fixed: the `has_successful_pr_review` skip path no longer bypasses the asserted_unavailability guard (6c) or the assert-grounded guard (6d).
- `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the broader argument that prompt-level enforcement drifts under load; this doc is a specific instance of that pattern.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — prior architect-discipline doc covering disposition-keyword paraphrasing; same agent, different failure surface.
- `crates/mika-agent/src/agent/` — required-tools-gate implementation in the EndTurn post-condition chain.
- `mika-platform` memory: `feedback_prompt_enforcement_fragile.md` — the underlying meta-rule this doc is a specific instance of.

## Adjacent: Transport-Contract Failure (mika#890)

This doc tracks *evasion* — the gate didn't fire because the LLM rationalized around its trigger. An adjacent but distinct failure family is *transport-contract* — the gate fired correctly, but the post-correction turn was a thin pointer-summary instead of a self-contained response. Only the final `EndTurn` is persisted to `messages`; the substantive content from mid-loop `ToolUse` turns is lost. See mika#890 and its plan at `docs/plans/2026-04-29-006-fix-required-tools-gate-retry-self-contained-plan.md` for the fix (engine correction message + skill-prompt reinforcement). The two families share a guard (required-tools gate) but differ in failure mode: evasion = gate doesn't fire; transport-contract = gate fires but persisted output is incomplete.

## Adjacent: Mode 3 Contract-Fabrication-of-Prior-Output (mika#918, mika#927)

A third adjacent family — sharing the meta-pattern *fabrication-to-avoid-work* but at a different gate. Where evasion fabricates the *required tool's status* to avoid calling it, Mode 3 fabricates the *prior emission* to avoid producing one. The architect emits a syntactically degenerate response that satisfies the suffix-line guard (`Disposition: ITERATE`) and references findings/sharpenings/prior emissions that do not exist in the session. The labels are typically snake_case `<topic>_<context>_<action>` and read as compound-doc filenames rather than finding bodies. N=2 across mika#918 (2026-05-01, session `1918cb84-c3fc-4514-b0cb-e55ef4b99b19`) and mika#927 (2026-05-02, session `95e5e97c-1583-4694-9949-b6f9bfe7ea93`). See `mode-3-compound-doc-name-emission-heuristic-2026-05-02.md` for the recognition heuristic and operator-discipline rules. The two families share the architect agent and overlap in cognitive cause (load-induced corner-cutting) but differ in surface: evasion = skip the call; Mode 3 = fabricate the prior call's output. A "third manifestation" prediction at the *meta* level — fabrication-to-avoid as a unifying class — is now N=3 across these three docs (gate-evasion, transport-contract, Mode 3); promotion to a unifying compound is the natural next step if a fourth instance lands.
