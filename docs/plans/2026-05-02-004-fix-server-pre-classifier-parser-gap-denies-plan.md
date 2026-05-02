---
title: "fix(server): pre-classifier rejects markdown-content brief messages via blanket backtick/$( rejection"
type: fix
status: active
date: 2026-05-02
---

# fix(server): pre-classifier rejects markdown-content brief messages via blanket backtick/$( rejection

## Overview

The structural pre-classifier at `crates/mika-agent/src/server/permission_pre_classifier.rs` (shipped via PR #937) rejects any `mika ask --agent <peer> ...` command whose **command string contains a backtick or `$(` anywhere**, even when those characters are inside the quoted message argument as literal content (markdown inline code, technical prose). This blocks the canonical `/mika-ask-arch` invocation form whenever the message body is a markdown brief — which is the standard case for `/mika-groom-ticket` Phase 3 step 9 architect calls.

**Verified live during canary v7 on mika#931 (2026-05-02 18:25 UTC):**

- claude-pilot session: `4cfc594f-008a-4038-b132-618751c0f569`
- Two `[denied]` lines — diagnosed by reading both the pre-classifier source and the actual brief content:
  - Deny #1: `mika ask --agent mika-arch --format json --verbose "<brief-with-30-backticks>"` — brief at `/tmp/groom-brief-mika-931-pass1.md` contains **30 backticks** (markdown inline code spans like `` `docs/plans/<file>.md` ``, `` `plan-doc-check` ``).
  - Deny #2: `mika ask --agent mika-arch --format json --verbose "$(cat /tmp/groom-brief-mika-931-pass1.md)" 2>&1` — uses command substitution `$(...)` literally.
- Both denies traced to `pre_classify_pilot_event` branch 5 at `crates/mika-agent/src/server/permission_pre_classifier.rs:112`:
  ```
  if command.contains("$(") || command.contains('`') {
      return None;
  }
  ```
- The retry that ALLOWED (`mika ask --agent mika-arch "Review this plan..."`) succeeded because the model paraphrased the message without backticks. This is degraded — the JSON envelope and `session_id` capture are dropped, breaking Phase 4 of the two-pass groom.

**Important diagnosis correction from the ticket body:** The mika#938 ticket body asserts the bug is "flag injection between `<peer>` and `<message>`." That's incorrect. Inspecting `extract_peer_from_tokens` at `crates/mika-agent/src/server/permission_pre_classifier.rs:280`, the parser correctly handles flag injection via a token-scanning loop that finds `--agent` anywhere after `ask`. Comment at line 152 explicitly lists *"Flag reordering: `mika ask "msg" --agent mika-arch`"* as a supported case. The actual root cause is **branch 5's blanket character rejection**, not the parser's positional logic.

## Problem Frame

The pre-classifier's backtick/`$(` rejection is a security boundary — these characters in an unquoted shell context enable command substitution (arbitrary code execution). PR #937 chose blanket rejection as a defense-in-depth measure: better to fall through to the LLM classifier (and possibly deny) than risk a structural-allow on a command that could expand to something dangerous.

But the rejection is too broad. In a command like:
```
mika ask --agent mika-arch --format json --verbose "<brief with `inline code` and $(cmd) text>"
```

The backticks and `$()` inside the quoted string would be evaluated by Bash on actual execution — which IS a real concern. But the structural pre-classifier never executes the command; it just decides whether to short-circuit-allow vs fall through to the LLM. And the message is destined for `mika ask`, which receives it as a quoted argument and forwards it to the agent's input — the agent process never invokes a shell on the content.

The right boundary is: **reject backtick/`$(` only when they're OUTSIDE quoted regions** (where they'd actually trigger shell expansion when the relay's downstream tooling actually executes the command). Inside `"..."` or `'..."`, they're literal characters that get passed to `mika ask` as part of the message argument.

This is a parser correctness issue: the pre-classifier needs quote-aware scanning, not a blanket `String::contains`.

## Requirements Trace

- **R1.** The pre-classifier short-circuit-allows `mika ask --agent <peer> --format json --verbose "<message>"` when `<message>` is a markdown brief containing backticks and/or `$()` as literal content inside quoted regions.
- **R2.** The pre-classifier continues to reject backtick/`$()` when they appear OUTSIDE quoted regions (where they'd trigger real shell command substitution on execution).
- **R3.** The pre-classifier continues to reject TIER 3 patterns (`rm -rf`, `git push --force`, etc.) regardless of where they appear (inside or outside quotes) — TIER 3 is about pattern recognition, not shell metacharacter handling.
- **R4.** All 22 existing test fixtures from PR #937 continue to pass (no regression).
- **R5.** New test fixtures cover the canonical /mika-ask-arch forms with markdown-content messages: backticks inside quoted message, `$()` inside quoted message, both combined, equivalent on `mika-dev` and `mika-qa` peers.
- **R6.** New negative fixtures preserve the security boundary: backtick/`$()` outside quotes (e.g., `mika ask --agent mika-arch "msg" \`rm -rf /\``) still denies via the unquoted-region check.
- **R7.** Re-firing the dev-groom canary on mika#931 reaches three green criteria within 10 minutes, with zero `[denied]` on any /mika-ask-arch invocation.

## Scope Boundaries

- Only `crates/mika-agent/src/server/permission_pre_classifier.rs` (and its `#[cfg(test)]` test module) is modified.
- The blanket-rejection logic at `permission_pre_classifier.rs:112` is the only behavioral change. Other branches (agent_id check, prefix check, JSON parse, tool_name check, peer extraction, TIER 3 check, compound-command split, pipe handling) are NOT modified.
- TIER 3 pattern matching at line 117 (`contains_tier3_pattern`) keeps its blanket `String::contains` semantics — that's the right shape for those patterns and is out of scope here.
- The Python `tier1.py` mirror (sentinel cross-reference at `permission_pre_classifier.rs:60-65`) is NOT modified by this plan. Python and Rust may temporarily drift on this branch; the sentinel threshold (refactor to codegen if drift) doesn't escalate at N=1.

### Deferred to Separate Tasks

- **Companion fix in `claude-pilot-py/src/claude_pilot/tier1.py`** (per pass-1 F2, BLOCKING — pinned with explicit title + filing gate). The Python mirror keeps blanket-rejection (intentional asymmetry: tier1.py is the primary fast-path; stricter is safer there). Divergence documented in F5 sentinel comment in both files.
  - **Companion task title:** `fix(security): quote-aware metacharacter rejection in tier1.py to match permission_pre_classifier.rs (mika#938 follow-up)`
  - **Filing gate:** Companion ticket MUST be filed BEFORE this PR merges. The PR description includes a checkbox for the implementer to confirm the companion ticket is filed; reviewer blocks merge if it isn't.
- **Hardening the /mika-ask-arch invocation form** — could pass the message via stdin (`mika ask --agent <peer> -` reading the brief from stdin) instead of as a quoted argument. This eliminates the shell-quoting ambiguity entirely. Worth a separate ticket; not blocking this fix.
- **mika-relay TIER 1 prompt changes** — out of scope per Vincent-locked constraint (don't re-introduce wallpaper trap).

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/server/permission_pre_classifier.rs:91` — `pub fn pre_classify_pilot_event(user_message: &str, agent_id: &str) -> Option<PermissionAction>` — the entry point with 7 documented branches.
- `crates/mika-agent/src/server/permission_pre_classifier.rs:112` — branch 5, the over-broad rejection: `if command.contains("$(") || command.contains('\`')`.
- `crates/mika-agent/src/server/permission_pre_classifier.rs:136` — `fn contains_tier3_pattern(command: &str) -> bool` — uses `String::contains` semantically (TIER 3 patterns must be detected anywhere in the string; this is correct, not a parallel bug).
- `crates/mika-agent/src/server/permission_pre_classifier.rs:202` — `fn try_parse_mika_ask_dispatch(command: &str) -> Option<&'static str>` — handles the dispatch parsing, including flag injection via `extract_peer_from_tokens`.
- `crates/mika-agent/src/server/permission_pre_classifier.rs:355` — `fn advance_past_quoted(bytes: &[u8], start: usize) -> usize` — existing quote-handling helper used by `shell_tokenize`. Same byte-level approach can be adapted to scan-and-skip-quoted-regions for the new check.
- `crates/mika-agent/src/server/permission_pre_classifier.rs:375` — `fn shell_tokenize(input: &str) -> Vec<&str>` — uses `advance_past_quoted` to skip past quoted strings; precedent for quote-aware scanning at byte level.
- `crates/mika-agent/src/server/permission_pre_classifier.rs:416-758` — existing 22 test fixtures in `mod tests`. New fixtures slot in here.

### Institutional Learnings

- **PR #937** (mika#935 Unit 2) shipped the structural pre-classifier. The plan's F4 BLOCKING (parser corner cases) covered 22 fixtures across 6 corner-case categories. F4 missed: messages whose CONTENT contains shell metacharacters as literal markdown. The fixture set tested command structure (env-var prefix, compound separators, flag re-ordering, `--` separator, equals form, quoting variants) but not message content.
- **F5 sentinel** (`permission_pre_classifier.rs:60`) — Python (`tier1.py`) is canonical, Rust mirrors. The same gap exists in tier1.py and needs companion fix (filed as deferred task).
- **mika#935 RR-002** — per-peer prompt-injection guards (deferred follow-up). Note that backticks-as-prompt-injection is a different threat model from this fix (which is about parser correctness for legitimate markdown briefs); RR-002 stays orthogonal.
- **/mika-ask-arch skill spec** — invokes `mika ask --agent mika-arch --format json --verbose [--session-id <id>] "<message>"`. The `<message>` is universally markdown content (peer-review briefs, second-pass briefs). Markdown-with-backticks is the norm, not the exception.

### External References

None — this is a Rust parser correctness fix grounded entirely in the existing codebase and the canary v7 evidence.

## Key Technical Decisions

### Decision 1: Quote-aware scan for backtick/`$(` outside quoted regions

**Decision:** Replace the blanket `command.contains("$(") || command.contains('\`')` check at `permission_pre_classifier.rs:112` with a byte-level scan that walks the command string and skips contents of quoted regions (single-quoted `'...'`, double-quoted `"..."`). Reject only when the metacharacters appear in unquoted regions.

**Rationale:**

- The security boundary is shell command substitution. That requires the metacharacters to be in a position where Bash interprets them — outside quotes (or inside double quotes, where backtick still expands but `$(` does too). For STRICT correctness, double-quoted strings DO expand backticks/`$()` in Bash; only single-quoted strings are fully literal.
- However, in the context of `mika ask <args>`, the message argument is delivered to `mika`'s clap parser which receives the post-shell-resolved string. By the time `mika` sees the message, any `$()` or backticks have already been expanded by the SHELL RUNNING THE COMMAND — which means the relay's pre-classifier sees what the model emitted, BEFORE shell expansion. So the relay's job is to decide "would this command be safe to execute?" — and that requires recognizing whether the metacharacters WOULD expand on actual shell execution.
- For pragmatic safety: the pre-classifier should reject metacharacters in **any unquoted region OR inside double-quoted regions**. Reject only inside SINGLE-QUOTED regions and treat those as literal. This matches Bash's actual quoting semantics.
- Architect's call on whether to allow inside double-quotes too. If yes, /mika-ask-arch's existing form (which uses double quotes) works. If no, /mika-ask-arch must change to single-quote its message argument (a separate caller-side fix per "Deferred to Separate Tasks"). My read: allow inside double-quotes, because the model emitting `mika ask --agent <peer> "msg with \`code\`"` is the standard pattern AND because any actual shell-expansion attack vector requires the attacker to control the message content in a way that's already a prompt-injection problem (RR-002 territory). The pre-classifier is defense-in-depth; the LLM classifier and TIER 3 deny patterns remain.

**Rejected alternatives:**

- **Option A: Reject only in unquoted regions, allow inside both single and double quotes.** Cleaner code, but accepts that `mika ask --agent mika-arch "msg \`rm -rf /\`"` would structurally allow even though Bash WOULD expand the backtick. Defends only against unquoted metacharacters. Vincent's strict-on-security memory leans against this.
- **Option B: Reject in unquoted AND double-quoted regions; allow only in single-quoted.** Most conservative. Forces /mika-ask-arch to single-quote its message. Cleanest security boundary. Heavier caller-side change.
- **Option C (chosen): Reject in unquoted regions; allow inside any quoted region (single or double).** Pragmatic: matches the de-facto invocation pattern and accepts that double-quoted backtick is a degraded-security case mitigated by RR-002 + LLM-tier defense. Architect's pass-1 should ratify or escalate.

### Decision 2: Apply the same quote-aware logic to TIER 3 pattern check OR keep blanket?

**Decision:** Keep TIER 3 pattern check (`contains_tier3_pattern` at line 136) as blanket `String::contains`. Do NOT make it quote-aware.

**Rationale:** TIER 3 patterns are dangerous regardless of quoting. `rm -rf` inside a quoted message that gets piped or `eval`d elsewhere is still a smell. The conservative posture for TIER 3 is to fall through to the LLM and let it deny — better than structural-allow on anything containing `rm -rf` text. This is consistent with the F5 sentinel discipline (TIER 3 patterns mirror tier1.py exactly).

### Decision 3: Helper function shape — new function vs. inline

**Decision:** Extract a new helper function in `permission_pre_classifier.rs` (e.g., `contains_unquoted_metacharacter`) that takes the command string and returns true iff `$(` or backtick appears outside single-quoted regions (and outside double-quoted regions per Decision 1, Option C). Replace the inline check at line 112 with a call to this helper.

**Rationale:**
- Testable in isolation (covered in Unit 1's test scenarios).
- Reuses the `advance_past_quoted` helper at line 355 — same byte-walking precedent.
- Names the security intent (the function name says what it checks).

## Open Questions

### Resolved During Planning

- **Is the bug really flag injection (per ticket body) or backtick rejection (per source inspection)?** → Backtick rejection. Confirmed via reading `extract_peer_from_tokens` at line 280 (handles flag injection) and tracing the canary log to branch 5 at line 112 (rejects on backtick/`$(`). Ticket body diagnosis was wrong; this plan corrects it.
- **Should the rejection logic preserve security inside double-quoted regions?** → Architect's call (Decision 1). Plan picks Option C (allow inside any quoted region) with rationale; alternatives B (allow only in single-quoted) and A (allow inside both, reject only unquoted) are documented as rejected.
- **Companion fix in tier1.py?** → Deferred to separate task. Same gap exists in Python mirror; mirror after canary verifies the Rust fix.

### Resolved by pass-1 architect

- **F1 (BLOCKING) — escape handling pin.** The byte scanner MUST treat `\"` inside a double-quoted region as an escaped quote: quote state does NOT toggle when `\"` is encountered. The scanner advances past the backslash-quote pair atomically. Symmetric rule for `\'` inside single-quoted regions. Rationale: the de-facto /mika-ask-arch invocation form uses double quotes with escaped inner quotes for nested code references; the alternative (don't track escapes; always toggle on `"`) would conservatively-reject legitimate briefs and force caller-side change. Architect ratified Option C (allow inside any quoted region) at pass-1; escape-aware quote tracking is the consistent extension. Mandatory fixture: `mika ask --agent mika-arch "has \"escaped\" and \`backtick\`"` → `Some(Allow)`.

### Deferred to Implementation

- **Exact byte-walking implementation of `contains_unquoted_metacharacter`** — derive during /ce:work using `advance_past_quoted` as the precedent. The escape-handling rule is now LOCKED per F1 above (escape-aware: `\"` and `\'` do not toggle quote state). Implementation must also handle: empty string, mixed quote types (`'a"b'` is single-quoted including the literal `"`), unterminated quotes (per Decision-equivalent: conservative reject — if a quote opens and never closes, fall through to LLM).

## Implementation Units

- [ ] **Unit 1: Quote-aware metacharacter rejection helper**

**Goal:** Replace the blanket `command.contains("$(") || command.contains('\`')` at `permission_pre_classifier.rs:112` with a quote-aware check that allows literal metacharacters inside quoted regions per Decision 1.

**Requirements:** R1, R2, R3, R4.

**Dependencies:** None.

**Files:**
- Modify: `crates/mika-agent/src/server/permission_pre_classifier.rs` (lines 110-114; add new helper function near the existing `contains_tier3_pattern`)
- Test: `crates/mika-agent/src/server/permission_pre_classifier.rs` (existing `#[cfg(test)] mod tests` at line 416; add new fixtures)

**Approach:**

1. Add a new helper function `fn contains_unquoted_metacharacter(command: &str) -> bool` near line 136 (alongside `contains_tier3_pattern`).
2. The helper walks the command bytes left-to-right, tracking quote state (none / single / double). Increments past `\\` escape sequences to handle escaped quotes. Returns `true` on first occurrence of `$(` or `` ` `` while in `none` quote state. Per Decision 1 Option C: returns `false` if those characters appear inside either single OR double quoted regions.
3. Replace line 112 with `if contains_unquoted_metacharacter(command) { return None; }`.
4. Update the doc-comment at line 88 (Branch 5 description) to reflect the new semantic: *"Branch 5: Command contains `$(` or backtick OUTSIDE quoted regions → None (would trigger shell command substitution on execution)."*
5. The F5 sentinel comment block at line 60 needs a note that branch 5 has diverged from `tier1.py`'s blanket form — refactor threshold (codegen) NOT triggered at N=1 divergence; pointer to the deferred companion fix on tier1.py.

**Patterns to follow:**

- `advance_past_quoted` at `permission_pre_classifier.rs:355` — existing byte-level quote-aware helper used by `shell_tokenize`. Mirror its handling of single/double quotes.
- `contains_tier3_pattern` at line 136 — pattern for a small focused check function. Naming convention (`contains_*`) and module placement.
- Existing test fixture style at line 416+ (`#[test] fn ...`) — keep new tests in the same module with similar naming.

**Test scenarios:**

| Category | Scenario |
|---|---|
| Happy path | `mika ask --agent mika-arch --format json --verbose "Brief with \`inline code\` and \`docs/plans/file.md\`"` (markdown brief inside double-quotes, 2 backticks inside quoted region) → Some(Allow). |
| Happy path | `mika ask --agent mika-arch --format json --verbose 'Brief with $(literal) text'` (single-quoted message containing `$(`) → Some(Allow). |
| Happy path | `mika ask --agent mika-arch --format json --verbose --session-id abc-123 "Second-pass review with \`session_id\` reference"` (combined flags + backtick in quotes) → Some(Allow). |
| Happy path | Equivalent forms on `mika-dev` and `mika-qa` peers with the same backtick-in-quoted-message shape → Some(Allow). |
| Edge case (F1 mandatory) | `mika ask --agent mika-arch "has \"escaped\" and \`backtick\`"` (escape-aware quote tracking — `\"` does not toggle state, backtick remains inside double-quoted region) → Some(Allow). Verifies F1 escape-handling pin. |
| Edge case | `mika ask --agent mika-arch "msg with \"escaped quote\" and \`outer-quoted backtick\`"` (escaped inner quotes; backtick still inside double-quoted region) → Some(Allow). |
| Edge case | Empty message: `mika ask --agent mika-arch ""` → Some(Allow) (peer match works; no metacharacter to evaluate). |
| Edge case | Unterminated quote: `mika ask --agent mika-arch "unterminated message with \`backtick\` and no closing quote` (no trailing `"`) → None (conservatively reject; the parser cannot determine the quoted region's end so falls through to LLM). |
| Error path | `mika ask --agent mika-arch --format json --verbose "msg" \`rm -rf /\`` (backtick OUTSIDE the quoted message; would actually expand on shell execution) → None (still falls through to LLM, which should TIER 3 deny on the `rm -rf` substring anyway). |
| Error path | `mika ask --agent mika-arch "msg" $(rm -rf /)` (`$(` outside quoted region) → None. |
| Error path | TIER 3 inside quoted region: `mika ask --agent mika-arch "msg with rm -rf / inside quotes"` → None (TIER 3 pattern check at line 117 still triggers on the `rm -rf` substring per Decision 2; this is intentional). |
| Integration | All 22 existing test fixtures from PR #937 in the same `#[cfg(test)] mod tests` continue to pass without modification. |
| Integration | E2E: re-fire dev-groom canary on mika#931 (`mika ask --agent mika-dev "groom mika issue#931"`). Three green criteria reach within 10 minutes, claude-pilot session log shows zero `[denied]` lines on any /mika-ask-arch invocation form. |

**Verification:**

- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all` — 22 existing + new fixtures all pass; total fixture count increases by at least 12 (the table above lists 12 new scenarios after F1's mandatory escape fixture; implementer may split or add).
- `git diff` shows changes confined to `crates/mika-agent/src/server/permission_pre_classifier.rs` only (Unit 1 surface).
- The F5 sentinel comment at line 60 is updated to note the Python/Rust divergence on branch 5 with pointer to the companion task title pinned in Scope Boundaries → Deferred to Separate Tasks.
- The doc-comment at line 88 (Branch 5 description) reflects the new semantics.

- [ ] **Unit 2: Compound doc — test-fixture content coverage discipline**

**Goal:** Author `docs/solutions/best-practices/test-fixture-content-coverage-2026-05-02.md` capturing the N=2 pattern: test fixtures for permission classifiers must cover both **command structure** (env-var prefix, compound separators, flag placement, etc.) AND **representative message content** (markdown briefs with backticks, code spans, technical prose). Per pass-1 F3 (BLOCKING) — N=2 threshold reached, compound doc authors NOW, not via forward-pointer.

**Requirements:** Pass-1 F3.

**Dependencies:** Unit 1's diagnosis must be locked (it is, after pass-1).

**Files:**
- Create: `docs/solutions/best-practices/test-fixture-content-coverage-2026-05-02.md`

**Approach:**

The compound doc names two grounding cases (N=2):

- **Case 1 — PR #937 (mika#935 Unit 2):** 22 test fixtures covered command structure across 6 corner-case categories (env-var prefix, compound separators, flag re-ordering, `--` separator, equals form, quoting variants). Fixtures used short ASCII messages (`"Test"`, `"hello"`). Result: a corner case bit in production canary v7 — markdown briefs with backticks triggered the blanket rejection that fixtures didn't surface.
- **Case 2 — mika#938 (this plan):** The remediation. Adds 12+ fixtures covering message-content variants (backticks inside double-quoted, `$()` inside single-quoted, escaped inner quotes, etc.).

The pattern: **fixtures must include representative production message-content shapes, not just structural variants of the command form.** For permission classifiers specifically, the "production shape" includes markdown briefs with inline code, file paths in backticks, and technical prose — the canonical /mika-ask-arch invocation.

The doc names a generalized testing discipline:

> *"When testing parsers/classifiers that process user-controlled CONTENT inside structured commands, fixtures must enumerate both:*
> *(a) Command-structure variants — flag ordering, compound separators, quote types, env-var prefixes;*
> *(b) Content-payload variants — representative production message shapes including their characteristic metacharacters (markdown backticks, dollar-paren in code spans, escaped quotes, etc.).*
> *Skipping (b) leaves the parser exposed to corner cases that only surface when real production traffic flows through."*

Forward-pointer for the next instance: when N=3 surfaces, append to this same doc rather than authoring a third compound entry.

**Patterns to follow:**

- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — same shape (compound doc capturing a parser/classifier discipline grounded in production evidence). Match the structure: short framing, two grounding cases with citations, generalized rule, escalation criteria.
- `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — N=1-to-N=2 progression pattern (started as a forward-pointer in a single ticket; promoted to a compound doc when the second instance surfaced). Mirror the citation discipline.

**Test scenarios:**

- Test expectation: none — pure documentation with no behavioral change.

**Verification:**

- `ls docs/solutions/best-practices/test-fixture-content-coverage-2026-05-02.md` exists in the worktree.
- Doc cites both grounding cases (PR #937 + mika#938) with specific evidence (canary v7 session ID, 22-fixture count, the actual failing form).
- Doc names the generalized rule (a + b above) explicitly.
- Doc carries forward-pointer for N=3 escalation (append vs. new doc).

## System-Wide Impact

- **Interaction graph:** This file is read at compile time only (Rust). Effect propagates via `cargo build` → binary rebuild → `make deploy` → next mika-agent restart picks up new logic. mika-relay (the only agent that calls `pre_classify_pilot_event` per Branch 1) sees new behavior on next session start.
- **Error propagation:** None affected. The pre-classifier still returns `Option<PermissionAction>`; callers handle `None` by falling through to the LLM classifier (unchanged path). No new error variants, no new failure modes — just a narrower False-rejection rate.
- **State lifecycle risks:** None. The function is pure (no side effects, no I/O); changing the rejection logic doesn't affect any persistent state.
- **API surface parity:**
  - `claude-pilot-py/src/claude_pilot/tier1.py` (Python mirror per F5 sentinel) has the same blanket-rejection logic and the same gap. Companion fix deferred per scope boundary; the F5 sentinel threshold is not crossed at N=1 divergence.
  - mika-relay's permission-policy LLM TIER 1 prompt (PR #936) still enumerates the bare form only — that's wallpaper that remains as third-tier defense; not modified per Vincent-locked constraint.
- **Integration coverage:** Canary on mika#931 re-fire is the single integration test. Unit tests with the 11 new fixtures cover the parser correctness. No mocks required — the function is pure.
- **Unchanged invariants:**
  - Decision branches 1-4 (agent_id check, prefix check, JSON parse, tool_name check) — unchanged.
  - Decision branch 6 (intra-platform dispatch peer extraction via `extract_peer_from_tokens`) — unchanged. Confirmed via inspection that flag injection is already handled correctly.
  - TIER 3 pattern matching (`contains_tier3_pattern` at line 136) — unchanged blanket-`String::contains` semantics per Decision 2.
  - Compound-command splitting and pipe handling — unchanged.
  - 22 existing test fixtures — unchanged behavior.
  - The F5 sentinel cross-reference structure — unchanged shape; only the divergence note added.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Quote-aware logic has its own corner cases (escaped quotes, mixed quote types, unterminated quotes) that introduce a new bug class. | 11 new test fixtures cover the standard cases; conservative posture on unterminated quotes (reject) keeps the security boundary stable. Architect pass-1 reviews fixture exhaustiveness. |
| Decision 1's Option C (allow backtick inside double-quoted regions) accepts a degraded-security case where double-quoted backticks would expand on actual shell execution. | Defense-in-depth: LLM classifier still runs, TIER 3 patterns still detected, RR-002 prompt-injection guards (deferred follow-up) provide per-peer authorization. Architect can escalate to Option B (allow only in single-quoted, force /mika-ask-arch to change) if double-quoted-backtick threat is unacceptable. |
| Python (`tier1.py`) mirror diverges from Rust on branch 5; F5 sentinel says refactor at N=1 divergence is premature. | Document divergence in F5 comment block, file companion task to mirror in Python. Codegen escalation threshold (>10 patterns OR persistent drift) not crossed at N=1. |
| Plan-doc-check hook fails on PR open because the plan path isn't cited in the PR body or commit. | The fix-#931 work in flight will harden the PR-writer; for this PR, manually cite the literal path `docs/plans/2026-05-02-004-fix-server-pre-classifier-parser-gap-denies-plan.md` in the PR body or a commit body to satisfy the hook. |
| The ticket body's diagnosis (flag injection) was wrong; this plan re-diagnoses to backtick rejection. If the architect or operator hasn't reviewed the corrected diagnosis, plan-vs-ticket scope will diverge silently. | Plan's Overview and Open Questions explicitly call out the ticket-vs-plan diagnosis correction. Architect pass-1 ratifies or counter-diagnoses. Vincent reviews at PR-merge time. |

## Documentation / Operational Notes

- **Rollout:** Standard Rust deploy. `cargo build --release` → `make deploy` → mika-agent restart. mika-relay picks up new logic on next session start (~automatic, no manual intervention).
- **Verification timeline:** After PR merges and `make deploy` completes, re-fire `mika ask --agent mika-dev "groom mika issue#931"` directly. Watch for three green criteria within 10 minutes + zero `[denied]` lines.
- **Pattern claim (N=2, compound doc IN PLAN per pass-1 F3 BLOCKING):** This is the SECOND instance of *"verification fixtures cover command-structure but not message-content; the corner case bites in production."* Compound doc authors as Unit 2 of this plan at `docs/solutions/best-practices/test-fixture-content-coverage-2026-05-02.md`. NOT a forward-pointer — N=2 threshold reached, per `compound_doc_timing_forward_vs_retroactive_groom`. Two grounding cases: (1) PR #937 — 22 fixtures covered structure but not content, surfaced via canary v7. (2) mika#938 — this plan, fixes the resulting blanket-rejection bug.
- **Companion follow-up for tier1.py:** Once this Rust fix verifies, file `senara-solutions/claude-pilot-py` ticket to mirror the quote-aware logic. Same shape, different repo. The F5 sentinel pointer at `permission_pre_classifier.rs:60` should be updated to reference the new ticket.
- **Ticket body correction:** mika#938's body asserts flag injection. After this plan ships, file a comment on mika#938 explaining the correct root cause for future readers. Do NOT silently rewrite the ticket body — the audit trail of the misdiagnosis is institutionally valuable.

## Sources & References

- **Ticket:** [mika#938](https://github.com/senara-solutions/mika/issues/938)
- **Surfacing canary:** mika#931 v7, claude-pilot session `4cfc594f-008a-4038-b132-618751c0f569`, log at `/var/log/claude-pilot/4cfc594f-008a-4038-b132-618751c0f569.log`, two `[denied]` lines verified.
- **Source inspection that corrected the ticket diagnosis:** `crates/mika-agent/src/server/permission_pre_classifier.rs:91` (entry), `:112` (over-broad rejection), `:152` (comment confirming flag injection IS handled), `:280` (`extract_peer_from_tokens` actually handles it correctly).
- **Predecessor blocker (resolved):** mika#935 / [mika PR #937](https://github.com/senara-solutions/mika/pull/937) — shipped the structural pre-classifier; this fix narrows its over-broad rejection. mika-platform#76 / PR #78 — fixed the Phase 1 step 4 interactive bug; canary v7 reached Phase 3 step 9 because of that fix.
- **Diagnostic confirmation:** mika-relay queried 2026-05-02 ~18:30 UTC confirms TIER 1 prompt enumerates only bare `mika ask --agent mika-arch ...` (no flag variants); the fall-through deny path is consistent with the structural pre-classifier returning None on branch 5.
- **F5 sentinel:** `permission_pre_classifier.rs:60-65` — Python/Rust mirror discipline; refactor-to-codegen threshold.
- **Related institutional knowledge:**
  - `feedback_compound_infra_fixes.md` — infra fixes evaporate faster than product fixes; compound at N=2.
  - `compound_doc_timing_forward_vs_retroactive_groom` — N=2 threshold.
  - `feedback_mika_dev_llm_fabricates_tool_errors.md` — same family as this canary's mika-dev "relay deny" misdiagnosis (orthogonal symptom).
