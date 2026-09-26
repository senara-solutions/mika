# Plan: milestone-close-claim guard parity with #483 — tighten regex + match milestone-number in cross-reference (mika#1207)

type: bug (engine post-condition guard)
ticket: mika#1207
date: 2026-05-19
groomed-via: peer-Claude staff-engineer review (3 rounds; mika-arch blocked on milestone-workflow class by this very ticket)

## Problem

mika#1207 is the engine-side false-positive in the `milestone-close-claim guard` (post-condition 4b on EndTurn — see `crates/mika-agent/CLAUDE.md` § Post-Conditions). The guard already has the keyword-then-tool-call structural shape from #483's completion-claim-guard work, but in two weaker forms than #483 ships: (1) the keyword regex `\bmilestone\b.{0,80}\b(closed|close)\b` matches *third-person planning-shape prose*, not just first-person assertion claims; (2) the cross-reference suppresses on *any* milestones-path PATCH call in the turn, not on a PATCH that matches the *specific milestone number* the agent claims.

Today's incident (2026-05-19): mika-arch's iterate-on-brief response for mika#789 contained phrases like "the plan proposes mika-dev call `gh api PATCH /repos/.../milestones/N` to close the GitHub milestone." Regex matched (third-person planning), no `run_gh` in turn (mika-arch is read-only by design), guard fired, mika-arch spent her turn explaining the misfire instead of completing the review. **Net effect: mika-arch is structurally unable to review milestone-workflow tickets — precisely the class her architect role is most needed for.** This fix is a parity-with-#483 pattern carry, not new infrastructure.

## Context

- **Per-turn `tool_calls` visibility is settled by precedent.** The #483 completion-claim guard (`docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md`) fires inside `run_loop()`'s EndTurn handler with per-turn local visibility on the current turn's tool-call summaries. The milestone-close-claim guard at `crates/mika-agent/src/agent.rs:4761` lives in the same call site and takes the same `&[ToolCallSummary]` slice. No visibility-scope question to resolve.

- **The current code already implements option (b) — partially.** `detect_milestone_close_claim_without_patch(text, all_tool_summaries)` at agent.rs:4761 already iterates summaries and suppresses on a satisfying `run_gh` PATCH call (criteria at agent.rs:4775: `name == "run_gh"`, `input_summary` contains `"api"`, `"PATCH"`, `state=closed`, matches `MILESTONE_API_PATH_RE`). So this isn't "add the cross-reference" — it's "tighten what already exists."

- **Two parallel weaknesses, each insufficient alone to fire today's incident.** (a) The regex is loose enough to match third-person planning-shape prose (intentionally documented as `_future_tense_overmatches_intentionally` in the inline tests at agent.rs:7954-8014). (b) The cross-reference URL match (`MILESTONE_API_PATH_RE` = `/repos/[^/]+/[^/]+/milestones/\d+` at agent.rs:4741) doesn't constrain the milestone number. mika-arch reviewing a brief about milestone#789 could write "to close the milestone" (regex hit) while having PATCHed milestone#14 in the same turn (cross-reference suppresses).

- **#483's discrimination granularity is correct for #483's domain, wrong for milestone-close.** #483 uses presence/absence on `update_work_item_status`: any call satisfies, regardless of work item ID. That's correct for #483. The milestone variant is different: mika-arch can have multiple milestone PATCHes and multiple milestone references in a single turn, and the discrimination must be *which milestone is being claimed-closed vs which is being PATCHed*. Milestone-number matching is a deliberate divergence from #483's pattern, justified by mika-arch's multi-milestone review surface.

- **Existing test coverage (#797).** `tests/eval/grounding_regressions/milestone_close.rs` has C1 (PATCH + claim → pass) and C2 (claim without PATCH → fire). Inline tests at agent.rs:7954-8014 add: case-insensitive matching, unrelated "close" doesn't match, readback alone insufficient. None exercise third-person planning prose. None exercise cross-milestone claims.

- **Composition with #4 (completion-claim guard) is preserved.** Per CLAUDE.md § 4b: "Composes with #4: when both regexes match (e.g., 'completed and closed'), #4 fires first; #4b fires on a subsequent EndTurn after #4 is satisfied." The tightened first-person regex still matches "I completed milestone#14" and "I closed milestone#14" — composition unaffected. Test C6 verifies on a two-sentence shape.

## Acceptance criteria

- **AC1 — Primary regex tightens to first-person verb + "milestone", no number capture.** Rewrite `MILESTONE_CLOSE_CLAIM_RE` at `crates/mika-agent/src/agent.rs:4734-4738` from `\bmilestone\b.{0,80}\b(closed|close)\b` to `\b(I|we|i've|we've) (closed|closed out|completed)\b.{0,40}\bmilestone\b` (case-insensitive). One capture group (the verb phrase, existing shape). No number capture in this regex — milestone-number extraction is a separate concern, handled by AC3's helper.

- **AC2 — `MILESTONE_API_PATH_RE` gains a named capture for the milestone number, callsite shifts from `.is_match()` to `.captures()`.** Extend the regex at agent.rs:4741 from `/repos/[^/]+/[^/]+/milestones/\d+` to `/repos/[^/]+/[^/]+/milestones/(?P<num>\d+)`. Ripple at agent.rs:4775: the existing `MILESTONE_API_PATH_RE.is_match(&s.input_summary)` boolean check becomes `MILESTONE_API_PATH_RE.captures(&s.input_summary)` to extract the named `num` group for set-membership comparison (AC4). The PATCH-shape criteria (`"api"`, `"PATCH"`, `state=closed`) stay as substring checks; only the URL match shifts shape.

- **AC3 — Separate `extract_claimed_milestone_number` helper.** New helper:
  ```rust
  fn extract_claimed_milestone_number(claim_text: &str) -> Option<u64>
  ```
  Uses regex `milestone\s*#?\s*(\d+)` (case-insensitive) against `claim_text`, returns the first match's number parsed via `u64::from_str`. Callsite passes the slice starting at the primary regex's match position (so the helper searches the claim region, not earlier prose mentioning other milestones). Returns `None` when no parseable number appears in the claim region.

- **AC4 — Cross-reference uses set membership; falls back to presence/absence when claim has no number.** Refactor `detect_milestone_close_claim_without_patch`:
  1. Run primary regex (AC1) — if no match, return `None`.
  2. Call AC3 helper with the slice starting at the regex match.
  3. Collect PATCH milestone numbers: iterate `all_tool_summaries`, keep entries passing the existing PATCH-shape substring checks, extract `num` via AC2's named capture, collect into `HashSet<u64>`.
  4. If AC3 returned `Some(claimed_num)`: suppress (return `None`) iff `claimed_num ∈ patched_set`. Otherwise fire.
  5. If AC3 returned `None` (no parseable number in claim): fall back to presence/absence — suppress iff `patched_set` is non-empty. Justification: a no-number claim is ambiguous enough that the conservative behavior is right; tightening would force EndTurn corrections on hallucinations the LLM can't ground in a specific milestone, which doesn't help grounding.

- **AC5 — Retry semantics preserved.** Single retry via existing `milestone_close_claim_retry_done` flag (agent.rs:841, 1335-1339, 1384-1385). No new retry state.

- **AC6 — Test scenarios added to `tests/eval/grounding_regressions/milestone_close.rs`**:
  - **C3 — review prose with API path passes.** Agent (mika-arch identity) text: `"the plan proposes mika-dev call gh api PATCH /repos/owner/repo/milestones/789 to close the milestone"`; zero `run_gh` calls in turn → guard does NOT fire (no first-person verb).
  - **C3b — review prose without API path or number passes.** Agent text: `"the plan proposes mika-dev close the milestone after merge"`; zero `run_gh` calls → guard does NOT fire. Tests the first-person constraint blocks third-person planning prose even when no specific number or API path appears.
  - **C4 — first-person false-claim still catches.** Agent text: `"I closed milestone#14"`; zero `run_gh` calls → guard fires (mika#797 regression class preserved).
  - **C5 — cross-milestone claim, same turn.** Agent text: `"I closed milestone#14"`; `run_gh PATCH /repos/owner/repo/milestones/789` in turn → guard fires. Discriminator made concrete; presence/absence would suppress, milestone-number matching catches.
  - **C6 — composition with #4 preserved.** Agent text: `"I completed milestone#14. I closed it on GitHub."`; verify #4 fires first on "completed" (without `update_task_status`), then on retry #4b fires on "closed" (without `run_gh PATCH`). The two-sentence shape keeps both clauses inside the narrow first-person regex while exercising the sequential composition documented in CLAUDE.md § 4b.
  - **C7 — claim satisfied by mixed PATCH set.** Agent text: `"I closed milestone#14"`; same turn does `run_gh PATCH /repos/owner/repo/milestones/14` AND `run_gh PATCH /repos/owner/repo/milestones/789` → guard does NOT fire. Verifies AC4's set-membership semantics: the claimed number being in the set is sufficient; extra PATCHes don't trigger a false-positive.

- **AC7 — Inline unit tests in `crates/mika-agent/src/agent.rs:7954-8014` updated.** Rename or replace `_future_tense_overmatches_intentionally` to assert third-person/planning-shape does NOT match the new regex. Add inline tests for `extract_claimed_milestone_number`: with-number ("I closed milestone#14"), without-number ("I closed the milestone"), multiple-numbers-takes-first ("I closed milestone#14 not milestone#789"), hash-optional ("milestone 14"), spacing variants ("milestone # 14", "milestone#14", "milestone 14"). Add inline tests for the PATCH-set extraction: single-PATCH, multiple-PATCHes, no-PATCHes, malformed-URL-skipped, PATCH-shape-without-milestones-path skipped.

- **AC8 — Update `crates/mika-agent/CLAUDE.md` § 4b with a behavior note.** Add to the milestone-close-claim guard description:

  > *Discrimination granularity: when the claim contains a parseable milestone number, suppress only if that specific number appears in a PATCH URL within the turn; otherwise fall back to presence/absence. This is a deliberate divergence from #483's presence/absence pattern, justified by mika-arch's multi-milestone review surface — a single turn may legitimately PATCH one milestone while writing prose about another.*

  Note: describes behavior, not helper function names. Future renames of the helpers don't require a doc update. If § 4b doesn't currently exist with that exact heading, the implementation should accept whatever the existing heading is rather than forcing a rename.

- **AC9 — Lint and test gates.** `cargo clippy -p mika-agent --all-targets -- -D warnings` clean. `cargo fmt --check` clean. All existing eval suite tests pass plus the 6 new C3/C3b/C4/C5/C6/C7 scenarios + inline helper tests.

## Out of scope

- **mika#1182** (argv check hardening) — sibling guard concern, separate ticket, no bundling.
- **mika#1183** (close coverage gaps) — sibling, separate ticket.
- **Pre-fetch of milestone state via GitHub API** — out of scope. The guard is a post-condition local check, not a grounding-via-external-call. Don't expand the contract.
- **Spelled-out numbers** ("milestone fourteen") — acceptable hallucination-resistance edge case; LLMs don't reliably emit spelled-out numbers in technical context.

## Where remaining uncertainty sits (stated leans for the pipeline)

1. **First-person verb list completeness.** Current proposal: `(I|we|i've|we've) (closed|closed out|completed)`. Real hallucination shapes include "I've gone ahead and closed milestone#N" (matches) and "milestone closure confirmed" (no first-person subject — would miss). The cross-reference catches the latter (no PATCH → guard fires). **Lean: keep narrow on verbs**, let cross-reference catch nouns.

2. **40-char window vs alternates.** Original is 80, proposing 40. Real claim shapes ("I closed milestone#14" = 15 chars, "we completed milestone#15 today" = 24 chars, "I've closed out the milestone for mika#789" = 30 chars) all fit. **Lean: 40 chars**; 30 (tighter) and 50 (safer) are both defensible. Pipeline can adjust at implementation if needed.

## Related

- mika#797 — original milestone-close guard introduction
- mika#483 — completion-claim guard (precedent for the keyword + tool-call cross-reference shape; this fix brings the milestone variant to parity)
- mika#1182 — sibling: harden milestone-close guard's argv check (separate scope)
- mika#1183 — sibling: close coverage gaps for the guard (separate scope)
- `crates/mika-agent/src/agent.rs:4734-4784` — guard implementation site
- `crates/mika-agent/src/agent.rs:7954-8014` — inline test site
- `tests/eval/grounding_regressions/milestone_close.rs` — eval scenario site
- `docs/solutions/architecture-patterns/completion-claim-guard-work-item-state-enforcement.md` — #483 precedent doc
- `crates/mika-agent/CLAUDE.md` § Post-Conditions § 4b — guard's architectural docs (AC8 update target)
