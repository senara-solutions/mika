---
title: "fix: make plan-on-branch load-bearing in implementation + QA"
type: fix
status: active
date: 2026-04-26
origin: ~/.claude/plans/i-want-you-to-graceful-walrus.md (operator-authored, peer-reviewed across 3 cycles, mika-arch session 8ad65309-1c8c-4a5e-a370-cae75edd830b)
---

# Plan: make the plan-on-branch load-bearing in implementation + QA

## Context

**The problem.** mika-platform#54 (Ticket 1 of `slash-command-coherence` milestone) shipped a fraction of its groomed plan's acceptance criteria. The plan specified an 11-field `--verbose` metadata block (alphabetical in JSON, importance-ordered in text). What shipped: JSON `--verbose` is silently ignored (zero metadata), text mode emits only `session_id` (1 of 11). mika-qa approved with `VERDICT: hold[review]`, but the hold was for an unrelated cross-repo deployment-skew gate — she **never ran the binary against the AC**.

**Why this matters.** We invested significant grooming discipline this session (pre-commit discovery, two-pass architect review, gated-merge cross-repo discipline, plan-on-branch as the canonical contract). The empirical "Opus-grooming-survives-implementation" hypothesis from yesterday's handoff is N=1 against. The plan-on-branch isn't load-bearing in either pipeline.

**Diagnosis is BOTH/AND, refined per mika-arch session `8ad65309-...` review of this plan:**

1. **Real architectural conflict during implementation.** PR #54's body explicitly documents: "P0: Removed `--format json` — JSON mode nests it inside a `metadata` object, **breaking the two-pass pipeline**." `/mika-groom-ticket`'s parser scans for `session_id: <uuid>` lines on stdout; the original plan's JSON-nested-metadata shape would have broken that parser. This conflict is real, not fabricated. Grooming missed it — mika-arch (architect) approved the plan without grepping for existing consumers of the affected output channel. **This is a grooming-surface gap.**
2. **Conflict-resolution-without-escalation during implementation.** When claude-pilot hit the conflict, it scope-reduced and shipped, documenting in the PR body. Correct response would have been `send_message` to operator, plan amendment, re-dispatch. **This is an implementation-surface gap.**
3. **claude-pilot wrote a NEW plan file** (`2026-04-26-004-feat-ask-verbose-flag-plan.md` — verified to exist on `mika` main via `ls`) explicitly capturing the reduced scope, then implemented against that. The new file normalizes "implementer overrides plan" as acceptable when there's a documented reason. **This is the smoking-gun artifact.**
4. **mika-qa never verified the original plan's AC behaviorally.** Step 3e was silently skipped (gated on conditions #54 didn't meet). She approved with `VERDICT: hold[review]` for an unrelated cross-repo deployment-skew gate.

**The structural gaps that allowed all four observations above:**

1. **`/mika` re-derives the plan instead of consuming the existing one.** The current pipeline at `mika/.claude/commands/mika.md:57` is `/ce:plan $ARGUMENTS` (where `$ARGUMENTS` becomes the issue title+body) → `/ce:work`. `/ce:plan`'s SKILL.md (Phase 0.1) has a "Resume Existing Plan Work" branch but only fuzzy-matches on "obvious recent matching plan in `docs/plans/`" — kimi-k2.5 ignores it. `/ce:work` already accepts a plan path as input ("Plan doc path or description of work" per its `argument-hint`); `/mika` just doesn't pass one. Net effect: claude-pilot re-plans from issue prose, drift surface is the entire plan-derivation step.

2. **`qa-review` checks plan existence, not plan content.** `qa-review/system_prompt.md:80-84` validates that a plan file *exists* in the diff. `:189` extracts ACs from the *GitHub issue* (not the plan). Step 3e (build verification) at `:175-180` is **conditional** on the PR having "a linked GitHub issue with backtick-wrapped `mika` commands." mika-platform#54's issue body doesn't match that shape, so Step 3e was silently skipped. AC-failure verdict in `qa-review-build-callback/system_prompt.md:44` is `hold[review]` (advisory), not `block[ac]` (gating). Sonnet upgrade doesn't close this — the step is optional and the skill silently skips when conditions aren't met.

3. **No escalation path when implementation hits architectural conflict with the plan.** Today, when claude-pilot finds an AC that conflicts with reality (e.g., breaks a downstream parser), the only paths are: scope-reduce silently (what happened), or block indefinitely (no progress). Both are wrong. Correct path: `send_message` to operator naming the conflict, pause for plan amendment, re-dispatch. This convention is named in mika-platform#52's framing-divergence ESCALATE pattern but is not encoded as a structural gate anywhere.

**The intended outcome.** Both pipelines treat the plan-on-branch as the load-bearing contract:
- claude-pilot consumes it directly via `/ce:work <plan-path>`, with explicit "this plan is the contract; do not scope-reduce; surface conflicts via send_message" framing.
- mika-qa reads it as the AC checklist and verifies each bullet behaviorally.
- AC failure = `block[ac]` (gating), with a named escalation path that routes to operator review (not indefinite block).

## Approach

Three structural changes batched into one PR on the `mika` repo (skills are engine-coupled there). Plus the queued self-dev M2 milestone-title fix already in `project_decisions_in_flight.md`.

### Change 1 — `/mika` skips `/ce:plan` when a plan-on-branch exists

**File:** `mika/.claude/commands/mika.md`

Replace the unconditional Step 1 (`/ce:plan $ARGUMENTS`) with a branching shape:

- **Detect plan-on-branch.** When an issue was fetched (existing flow at line 12), parse the issue body for the `> - **Plan:** \`<path>\` (committed on branch @ \`<sha>\`)` callout — same shape `/mika-groom-ticket` writes during grooming. The grooming convention is already established; this just teaches `/mika` to read it.
- **If the callout is present AND `<path>` exists in the worktree:**
  - **Skip `/ce:plan` entirely.** The plan was groomed by mika-arch; re-running plan-derivation is the drift surface.
  - **Run `/ce:work <plan-path>` directly.** Per `/ce:work` Phase 0: "Plan document (input is a file path to an existing plan) → skip to Phase 1." Phase 1 reads the plan and "treat[s] it as a decision artifact."
  - **Add explicit contract framing** in the prompt construction: "This plan was groomed and committed by the architect. It is the contract for this implementation. If any AC is unclear or unsatisfiable, **send_message** to mika-dev surfacing the ambiguity — do not silently scope-reduce. Do not write a new plan file in `docs/plans/`."
- **If callout is absent or path missing:** fall back to current flow (`/ce:plan $ARGUMENTS` → `/ce:work`).

This is a 15-line change in a single markdown file. The drift surface (`/ce:plan` re-derivation) disappears entirely when a plan-on-branch exists. `/ce:work` already has the consumption logic; we're just routing to it directly.

### Change 2 (folded into Change 3 per peer-review)

The original "prohibit new plan files mid-implementation" intervention is **folded into Change 3 as an implicit structural AC** (verifiable post-pipeline via `git diff main -- docs/plans/`). Standalone Change 2 was redundant — post-PR verification of an instruction already given in Change 1's prompt is what Change 3's structural-AC layer does anyway. Folding eliminates a separate moving part.

**Override mechanism for legitimate sub-plan cases:** if a new file in `docs/plans/` is genuinely needed (e.g., milestone implementation adding a sub-plan for a complex child unit), the new plan must include `parent_plan: <path>` in its YAML frontmatter, and the operator must sign off before implementation. Change 3's structural-AC check reads frontmatter and skips the prohibition for files with valid `parent_plan` metadata. This handles the legitimate case without weakening the default.

### Change 3 — `qa-review` reads the plan and verifies each AC behaviorally (gating)

**File:** `mika/skills/bundled/qa-review/system_prompt.md`

Insert a new **Step 2.5 (Plan-AC verification)** between the existing diff-review steps and the conditional Step 3e (build verification). Replace the current AC-extraction at `:189` (which reads the GitHub issue) with plan-driven AC extraction:

1. **Read the plan file.** Parse the issue body for `> - **Plan:** \`<path>\``. If no callout: `VERDICT: block[pipeline]` with "no plan callout — was this groomed via `/mika-groom-ticket`?". If callout present but file missing on branch: same block. (Today's behavior — line 80-84 — already checks existence; this extends to read.)
2. **Extract AC bullets.** Read the plan's `## Acceptance criteria` section. The plan template (per `/ce:plan` Phase 4.2) names this section explicitly; the bullets are markdown checkbox items.
3. **Classify each AC:**
   - **Behavioral**: testable by running the built binary (e.g., `mika ask --verbose --format json` should emit a metadata object).
   - **Structural**: testable by grepping the diff (e.g., "field added to struct X" → `git diff main -- <file> | grep -q "<field>:"`).
   - **Documentation**: testable by reading a file path (e.g., `docs/getting-started.md` updated).
   - **CI-deferred**: explicitly marked "no test regressions" or similar — defer to CI pipeline.

   **Implicit structural AC (always applied when plan-on-branch detected):** `git diff main -- docs/plans/` shows zero new files unless the new file's frontmatter includes `parent_plan: <path>` (override for legitimate sub-plan cases). Violation produces `block[ac]` with reason "Parallel plan file authored without parent_plan override; plan-on-branch is the contract." This is the absorbed Change 2.
4. **Verify each AC by class.** For Behavioral, build the project (existing `build_mika` callback) and execute the AC command(s) extracted from the plan. For Structural, run the grep. For Documentation, read the file and check for the documented surface.
5. **Compose the verification block** in the PR review body. Per AC bullet: `[✅] satisfied: <evidence>`, `[❌] unsatisfied: <expected vs actual>`, or `[⏭️] CI-deferred`.
6. **Verdict mapping (gating, not advisory):**
   - All ACs `✅` or `⏭️`: `VERDICT: pass` (subject to other diff-review checks).
   - Any AC `❌`: `VERDICT: block[ac]` with the unsatisfied bullets enumerated.
   - Plan unparseable, file missing, or AC section absent: `VERDICT: block[pipeline]`.

7. **Escalation path on `block[ac]`** (per mika-arch's Finding 3 review of this plan, session `8ad65309-...`):
   - `block[ac]` is gating BUT must include a named escalation. The QA review body, when emitting `block[ac]`, must include a "Plan amendment required:" section listing each unsatisfied AC and the conflict reason inferred from the diff (e.g., "AC X specifies JSON nested metadata; downstream consumer `/mika-groom-ticket` parses `session_id: <uuid>` lines on stdout — these conflict; plan needs amendment to one or the other").
   - mika-dev's verdict-handler reads this section and treats `block[ac]` differently from `block[ci]`: instead of auto-retry, it `send_message`s to operator with the conflict summary and pauses the work item.
   - This closes the "implementer overrides plan silently" failure mode: an AC mismatch becomes operator-visible immediately, and the resolution path (amend plan or amend AC) is taken explicitly.
   - Cite: convention from mika-platform#52 framing-divergence ESCALATE — when implementation hits conflict with spec, surface to operator, don't unilaterally resolve.

Step 3e (build verification) becomes the *implementation* of the Behavioral AC class, not a separate conditional gate. The "linked GitHub issue with backtick commands" condition (`:175-180`) is removed — ACs come from the plan, not the issue.

**Companion file change:** `mika/skills/bundled/qa-review-build-callback/system_prompt.md:44` — change AC-failure verdict from `hold[review]` to `block[ac]`. Verbatim: "If build or any AC fails, the maximum verdict is `block[ac]`" (was `hold[review]`).

**Tools check:** verify `mika/skills/bundled/qa-review/skill.toml` declares the necessary `required_tools` for: file-read on the plan, `build_mika` callback for behavioral verification, `run_gh` for diff inspection. Add any missing tools.

### Change 4 — bundle the queued self-dev M2 milestone-title fix

**File:** `mika/skills/bundled/self-dev/system_prompt.md:386`

Already documented in `~/.claude/projects/-data-workspace-mika-platform/memory/project_decisions_in_flight.md`. Single-line replacement:

```diff
- "command": ["milestone", "list", "--json", "number,title", "--jq", ".[] | select(.number==<n>) | .title"],
+ "command": ["issue", "list", "--milestone", "<n>", "--state", "all", "--json", "milestone", "--jq", ".[0].milestone.title"],
```

`gh` has no `milestone` subcommand — verified today by direct attempt. The replacement uses the existing `issue list --milestone --json milestone` shape (verified working against milestone#2). Bundle into the same PR — same skill family.

### Change 5 — `block[ac]` mapping in mika-dev's verdict-handler (promoted from deferred per peer-review)

**File:** `mika/skills/bundled/self-dev-webhook-qa/system_prompt.md`

Originally deferred as a process-level item; promoted into this PR because **Change 3 ships a gating verdict (`block[ac]`) that no handler currently routes correctly.** Without Change 5, `block[ac]` either (a) gets misrouted as `block[ci]` (auto-retry — semantically wrong; AC mismatch isn't transient), or (b) gets treated as unrecognized verdict (undefined behavior). Either failure mode breaks the entire escalation chain Change 3 depends on.

The handler updates needed:

1. **Recognize `block[ac]` as a distinct verdict class.** Today's webhook handler distinguishes `hold[review]` (Vincent attention, no auto-retry) from `block[ci]` (auto-retry once with `ci_fix_count` budget). Add `block[ac]` as a third class.
2. **Parse the "Plan amendment required:" section** from the QA review body. This section, written by Change 3's qa-review, names each unsatisfied AC and the inferred conflict reason.
3. **Route to operator review.** On `block[ac]`:
   - Emit `send_message` to operator with the conflict summary (the parsed "Plan amendment required:" content + a one-line "this is a plan-vs-implementation conflict; auto-retry would be wrong; resolution requires plan amendment OR AC rewording").
   - Update the work item: `status="blocked"`, metadata note "Plan amendment required (block[ac])". Do NOT increment any retry counter; do NOT dispatch claude-pilot.
   - Pause the milestone if the work item is part of one (existing milestone-pause path).
4. **No auto-retry path.** Unlike `block[ci]`, AC mismatches don't auto-resolve. The next dispatch must come from the operator after plan amendment.

This is ~30-50 lines of skill-prompt change. Mechanically isolated, but structurally required.

### Change 6 — mika-arch grooming discipline: verify downstream parsers (promoted from deferred per peer-review)

**File:** `mika/skills/bundled/mika-arch-second-review/system_prompt.md`

Scoped to second-pass review only. First-pass `mika-arch-groom-ticket` operates on the issue body before any plan file exists; parser-conflict checks don't apply at that stage. Second-pass review is where the architect approves the plan-on-branch, and where the parser-compatibility check belongs.

Originally deferred as a process-level compound; promoted into this PR because **without it, the plans we feed Change 3 will keep having parser conflicts to catch.** Change 3 becomes the safety net rather than the primary gate. Catching conflicts at grooming time is the cause-fix; catching them at QA time is symptom containment. Both compound, but the cause-fix should ship in the same PR as the symptom-containment.

mika-arch self-identified the gap (session `8ad65309-...`): she approved the original `2026-04-26-002` plan with JSON-nested-metadata-rendering without checking whether `/mika-groom-ticket`'s `session_id: <uuid>` parser handles nested JSON. Pattern: she didn't have a discipline for "verify downstream parsers when plan specifies new output format."

The skill-prompt update adds a **second-pass review step** (alongside existing pre-commit-discovery, criterion-replacement, and review-guide checks):

> **Output-format compatibility check (mandatory for plans introducing or changing output shapes):** When the plan specifies a new or changed output format for **any output channel with documented downstream parsers** — including but not limited to: tool/binary/CLI surfaces (`mika ask`, `mika status`, `gh`, `cargo`, custom CLI commands), structured logs (`mika.log.YYYY-MM-DD` consumed by audit family), persisted audit events (`audit_events` rows consumed by introspection tools), HTTP API responses (consumed by gateway, dashboard, A2A clients), or any other channel a downstream consumer parses — the second-pass review must:
>
> 1. **Identify downstream consumers.** Any code or skill prompt that parses the affected output channel. Use `grep`/`gh_read` to find all callers of the surface in `mika/`, `mika-skills/`, and `mika-platform/.claude/commands/`. Common consumers: slash commands (`/mika-groom-ticket`, `/mika-ask-arch`, audit family), other skill prompts that pipe output, downstream test harnesses, dashboard consumers of HTTP responses, log-parsing audit commands.
> 2. **Verify compatibility** of the proposed output shape against each consumer's parser. If a parser scans for `<key>: <value>` lines on stdout, a JSON-nested-only output breaks it. If a parser expects newline-separated UUIDs, a comma-separated list breaks it. Cite each consumer-vs-shape compatibility check explicitly in the second-pass review.
> 3. **Surface conflicts as ESCALATE findings.** A parser conflict is the same shape as a structural-dependency assumption that turns out wrong (mika#821 Finding 6, mika-platform#52 Finding 2). The pre-commit-discovery discipline applies: 30-second `grep` check resolves it, surfacing the conflict before the operator commits the plan.
>
> Same shape as mika#821's `LlmProvider` accessor verification or mika-platform#52's `idx_llm_calls_session` verification — extending the pre-commit-discovery discipline from "verify your assumptions about source code" to "verify your assumptions about downstream parsers."

This is ~20-30 lines of skill-prompt change. Aligns mika-arch's discipline with the verification rigor she's already established for source-code assumptions.

### PR description discipline (per mika-arch Finding 5 + peer-review Addition A)

The PR description must explicitly distinguish failure classes and name the multi-layer pattern this work addresses. Six changes ship; they group as:

- **Changes 1, 3, 5 fix the conflict-resolution-drift surface across three pipeline layers.** When implementation hits a real architectural conflict with the plan (e.g., AC specifies a shape that breaks a downstream parser), the current shipped path is "scope-reduce silently and document in PR body." The fix routes implementation through the plan-on-branch as contract (Change 1, implementation layer), adds behavioral AC verification with named escalation when conflict is detected (Change 3, QA layer), and updates mika-dev's verdict-handler to route `block[ac]` to operator review without auto-retry (Change 5, dispatch layer). Change 2 is folded into Change 3 as an implicit structural AC.
- **Change 6 fixes the cause one layer up: grooming.** mika-arch missed the parser conflict at second-pass review of the original plan because no discipline existed for "verify downstream parsers when plan specifies new output format." Change 6 adds the discipline. With it, conflict-shaped plans are caught at grooming time; without it, Changes 1-3-5 are catching them as a backstop.
- **Change 4 fixes fabrication-class drift, an unrelated bug.** When `run_gh` returns an error (e.g., the milestone subcommand doesn't exist), kimi-k2.5 fabricates plausible data rather than admitting the missing field. Different mechanism, different fix. Bundled because same skill family, single review cycle.

**Multi-layer-pattern note (peer-review Addition A):** This PR addresses three layers of a multi-layer verification failure pattern (grooming, implementation, QA + dispatch). The pattern of failures-passing-through-multiple-verification-points may recur in other classes; the eventual artifact for retroactive multi-layer drift detection is `mika-plan-audit` (deferred until a second incident motivates building it). The link is preserved here so future readers understand why grooming AND QA AND implementation skills were all touched in one PR.

Cite: north-star.md honest system description.

## Files

| Change | File | Diff shape |
|---|---|---|
| 1 | `mika/.claude/commands/mika.md` | Detect plan-on-branch callout (line 12-13 region); if present, skip `/ce:plan`, route to `/ce:work <plan-path>` with contract framing |
| 3 | `mika/skills/bundled/qa-review/system_prompt.md` | Insert Step 2.5 (plan-AC behavioral verification + implicit-structural-AC for no-new-plan-files-without-parent_plan-override); remove conditional gate at lines 175-180; replace issue-AC parsing at line 189 with plan-AC parsing |
| 3 | `mika/skills/bundled/qa-review-build-callback/system_prompt.md` | Verdict severity at line 44: `hold[review]` → `block[ac]` |
| 3 | `mika/skills/bundled/qa-review/skill.toml` | Verify `required_tools` covers plan-read + binary-execute (already callback-mediated) + diff-inspect |
| 4 | `mika/skills/bundled/self-dev/system_prompt.md:386` | Milestone-title fetch via `issue list --milestone --json milestone --jq '.[0].milestone.title'` |
| 5 | `mika/skills/bundled/self-dev-webhook-qa/system_prompt.md` | Recognize `block[ac]` verdict; parse "Plan amendment required:" section; route to operator via `send_message`; pause work item; no auto-retry |
| 6 | `mika/skills/bundled/mika-arch-second-review/system_prompt.md` | Add output-format-compatibility-check step: identify downstream consumers, verify shape, surface conflicts as ESCALATE findings |

Estimated diff: ~150-200 lines across 7 files (was 5). Single PR, single review cycle.

## Process-level changes (deferred — not part of this PR)

These remain out of scope for the PR. Items 1 and 4 from the prior version were promoted into Changes 5 and 6 respectively per peer-review.

1. **`mika-plan-audit` retroactive drift detection.** A read-only audit that diffs plan AC against shipped behavior on any merged PR. Useful for retroactively reviewing mika#816 / mika#824 / future drift. New tool surface — defer until we see a second drift incident post-this-fix. Also serves as the eventual artifact for "verification at every layer" if multi-layer-failure pattern recurs.

2. **mika-qa Sonnet upgrade.** Vincent already in flight. Independent of the structural fix; both compound.

3. **Grooming-discipline compound doc.** Once Change 6's skill-prompt update lands and proves out, file a compound doc in `mika/docs/solutions/best-practices/` capturing the pattern: "verify downstream parsers when plan specifies new output format" extends the pre-commit-discovery discipline from source-code assumptions to downstream-parser assumptions. mika-arch's own self-identification from session `8ad65309-...` is the primary citation. Compound after the fix lands so the doc references shipped code, not a hypothetical.

## Verification

### Primary acceptance criterion (peer-review Addition B)

**The fix is considered load-bearing if and only if mika#824's exact dispatch shape, replayed against the patched pipeline, produces `VERDICT: block[ac]` flagging the metadata-block AC as unsatisfied (with 10 of 11 specified fields missing from the shipped output). All other verification steps are subsidiary.**

Field-count nuance: the groomed plan has **9 acceptance criteria bullets**; one of those bullets specifies the metadata block ("`mika ask --verbose` emits the v1 metadata block in JSON and prose formats per the field list and rendering rules above") referencing **11 distinct metadata fields** (session_id, trace_id, task_id, agent_id, provider, model, started_at, completed_at, input_tokens, output_tokens, cache_read_tokens). The recurrence-test failure shape is "1 of 9 AC bullets unsatisfied, with 10 of 11 fields missing" — Change 3's verification block must list the AC bullet by name AND enumerate the missing fields in the evidence string.

Concretely: take the original groomed plan at `mika-platform/docs/plans/2026-04-26-002-refactor-mika-ask-verbose-metadata-plan.md` (committed @ `57ddea1` on branch `refactor/52/mika-ask-verbose-metadata` — verified to exist on that branch; not yet on `mika-platform` main because PR #54 is still open). If PR #54 has merged by the time this fix dispatches, the path is on main. If not, recover via:

```bash
git -C /data/workspace/mika-platform/mika-platform show refactor/52/mika-ask-verbose-metadata:docs/plans/2026-04-26-002-refactor-mika-ask-verbose-metadata-plan.md
```

File a fresh test ticket pointing to this plan content, dispatch via the patched mika-dev. Observe:

- claude-pilot reads the plan-on-branch (Change 1 working) — verifiable via session log showing `/ce:work <plan-path>` invocation, not `/ce:plan`.
- claude-pilot either (a) ships all 11 fields with **correct ordering (alphabetical in JSON, importance-ordered in text per the plan's rendering rules) and correct conditionality (token fields gated on `MIKA_STORE_LLM_CALLS=true`)** — success path; or (b) hits the same JSON-nested-metadata vs `/mika-groom-ticket`-parser conflict and `send_message`s to operator naming the conflict (Change 1 contract framing working).
- If claude-pilot scope-reduces silently anyway (Change 1 framing insufficient), mika-qa runs Step 2.5, executes `mika ask --verbose --format json "ping"`, observes the metadata-block AC unsatisfied (whether by missing fields, wrong ordering, or unconditional rendering of gated fields), emits `VERDICT: block[ac]` with the AC bullet identified by name AND the specific failure mode enumerated in the evidence string ("metadata block absent" / "fields not alphabetical" / "token fields rendered with `MIKA_STORE_LLM_CALLS=false`"), and includes a "Plan amendment required:" section identifying any underlying conflict if the failure was conflict-driven (Change 3 working).
- mika-dev's verdict-handler receives `block[ac]`, does NOT auto-retry, parses the "Plan amendment required:" section, and `send_message`s to operator with the conflict summary (Change 5 working).

**Verification asserts cover three failure classes** (per peer-review refinement on uncertainty #2):
1. **Presence:** the 11 fields exist in the rendered output (catches the actual mika#824 failure).
2. **Ordering:** alphabetical in JSON, importance-ordered in text per plan rendering spec (catches subtle drift where fields are present but mis-ordered).
3. **Conditionality:** token fields appear only when `MIKA_STORE_LLM_CALLS=true`, absent when `false` (catches drift where conditional gates are ignored).

Without all three classes asserted in the recurrence test, a PR could ship "10 fields present in the wrong order" and the test would falsely report load-bearing. Without this exact replay producing the expected gating across all three classes, **the fix is unproven regardless of other steps**. The previous-incident replay is the only evidence that the same failure shape can't recur.

### Subsidiary verification steps

These confirm the broader pipeline works but are not the proof obligation:

1. **Happy-path re-groom.** File a fresh test ticket with simple, unambiguous ACs, groom via `/mika-groom-ticket`, dispatch. Confirm `/mika` invokes `/ce:work <plan-path>`, claude-pilot ships all ACs, mika-qa emits per-AC verification block with all `✅`, verdict `pass`. Sanity check that the patched pipeline doesn't break ordinary tickets.
2. **Adversarial AC break.** In a fresh PR, manually remove one AC's implementation after claude-pilot completes. Confirm mika-qa returns `block[ac]` listing the unsatisfied bullet, and Change 5's verdict-handler routes to operator (no auto-retry).
3. **Adversarial parallel plan.** In a worktree with plan-on-branch, manually create `docs/plans/<new-file>.md` without `parent_plan` frontmatter. Confirm Change 3's implicit structural AC fires `block[ac]` with reason "Parallel plan file authored without parent_plan override."
4. **Override mechanism.** In a worktree with plan-on-branch, create a second plan file with valid `parent_plan: <path>` frontmatter. Confirm the override works — no false-positive block.
5. **Change 6 grooming verification.** Construct a test plan that introduces a new output format conflicting with an existing parser (the failure shape that produced #824). Run mika-arch second-pass review. Confirm Change 6's output-format-compatibility-check fires ESCALATE before approval.
6. **Change 4 milestone-title fetch.** Dispatch a fresh milestone via mika-dev. Confirm the milestone-title metadata reads the real title (not fabricated) and is sourced from the `issue list --milestone --json milestone` shape.

## Critical files (paths confirmed in Phase 1 exploration)

- `mika/.claude/commands/mika.md` — `/mika` per-repo dispatcher (line 12-13 issue-linking, line 57 `/ce:plan` invocation) — Change 1
- `mika/skills/bundled/self-dev/system_prompt.md` — self-dev (line 386 milestone-title fetch) — Change 4
- `mika/skills/bundled/qa-review/system_prompt.md` — QA review (line 80-84 plan-existence check, line 189 issue-AC parsing, lines 175-180 conditional Step 3e) — Change 3
- `mika/skills/bundled/qa-review-build-callback/system_prompt.md` — Build callback (lines 22-32 AC execution, line 44 verdict mapping) — Change 3
- `mika/skills/bundled/qa-review/skill.toml` — Tools manifest (verify behavioral-verification tools present) — Change 3
- `mika/skills/bundled/self-dev-webhook-qa/system_prompt.md` — mika-dev verdict-handler (path verified) — Change 5 (`block[ac]` mapping + "Plan amendment required:" parsing + operator routing + milestone-pause-on-block via existing `update_task_status status="blocked"` path at `self-dev/system_prompt.md:492`)
- `mika/skills/bundled/mika-arch-second-review/system_prompt.md` — mika-arch second-pass review skill (path verified) — Change 6 (output-format-compatibility-check step). Scoped to second-pass only; first-pass `mika-arch-groom-ticket` operates on issue text before the plan exists, so parser-conflict checks don't apply yet.

Reference (read during planning, not modified):
- `~/.claude/plugins/cache/every-marketplace/compound-engineering/2.65.0/skills/ce-plan/SKILL.md` — confirms `/ce:plan` is plan-derivation, not contract enforcement
- `~/.claude/plugins/cache/every-marketplace/compound-engineering/2.65.0/skills/ce-work/SKILL.md` — confirms `/ce:work` accepts plan-path input and treats it as decision artifact (Phase 1)

## Why one PR, not multiple

Per Vincent's "backlog must go down" constraint and the structural-interlock argument:

- All seven files are in the `mika` repo's bundled skills + per-repo command directory. Single worktree, single CI run, single review cycle.
- **Changes 1, 3, 5 form the implementation→QA→dispatch interlock for conflict-resolution drift.** Change 1 routes implementation to consume the plan-on-branch as contract. Change 3 verifies AC behaviorally and emits `block[ac]` with named escalation when conflict is detected. Change 5 routes `block[ac]` to operator review (no auto-retry). Each step is meaningless without the next: a verified gate that nothing routes correctly is just a misrouted gate; a contract framing without behavioral verification is just hopeful prose.
- **Change 6 is the cause-fix at the grooming layer for the same failure pattern.** Without it, plans keep landing with parser conflicts and Changes 1/3/5 keep firing as a backstop instead of as exception handling.
- **Change 2 is folded into Change 3** as an implicit structural AC ("no new files in `docs/plans/` without `parent_plan` frontmatter override").
- **Change 4 is unrelated fabrication-class drift.** Bundled because same skill family (`mika/skills/bundled/self-dev/`), single review cycle for the same reviewer.
- Splitting would multiply review burden and dispatch overhead with no architectural benefit. Six changes interlock or co-locate; none independently shippable except possibly Change 4.
- Working note in `project_decisions_in_flight.md` covers traceability for Change 4's bundled M2 fix.

Dispatch via `/mika` against the mika repo with free-text description pointing at this plan file; no new GitHub issue. The plan file at `~/.claude/plans/i-want-you-to-graceful-walrus.md` is the dispatch artifact.

## Open questions resolved during peer-review

1. **Enumerate CI-deferred ACs in the QA review block?** Yes, enumerate everything (✅/❌/⏭️). More honest, prevents invisible drift. Resolved.
2. **Hard error vs warning on plan-file prohibition?** Folded into Change 3 as an implicit structural AC with `parent_plan: <path>` frontmatter override for legitimate sub-plan cases. Standalone Change 2 dropped. Resolved.
3. **`mika-plan-audit` retroactive tool now or later?** Defer. Reactive tool for an ideally-rare failure mode; build when needed. Resolved.
4. **Granular `block[plan-empty]` vs lump under `block[pipeline]`?** Lump. KISS; split later if debugging volume justifies it. Resolved.

## Remaining uncertainties (not blocking)

These are real but don't gate plan approval:

1. **Whether Change 1's contract framing is sufficient to override kimi-k2.5's scope-reduction tendency.** Prompt-level instruction may not be enough; Change 3's behavioral verification is the explicit backstop. The recurrence test (Verification primary AC) is the proof: if claude-pilot still scope-reduces under Change 1's framing, Change 3 catches it and Change 5 routes to operator. The fix is layered; no single change has to be perfect.
2. **Whether Change 3's behavioral AC verification catches subtler invariant drift** (e.g., field present but populated at the wrong moment). Acceptable for v1; v2 would add invariant-spec ACs (e.g., AC could state `started_at MUST be captured before run_agent()`, verified by reading source). Defer.
3. **Whether Change 6's output-format-compatibility-check generalizes** to non-CLI surfaces (e.g., HTTP API contracts, structured log shapes). For now, scoped to "tool/binary/CLI surface" — broader generalization can land in a follow-up to mika-arch's discipline if a non-CLI conflict surfaces.
