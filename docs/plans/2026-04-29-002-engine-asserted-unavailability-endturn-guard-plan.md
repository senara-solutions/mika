---
title: "engine: asserted-unavailability EndTurn guard for tools claimed unavailable but in active registry"
type: engine
status: active
date: 2026-04-29
ticket: senara-solutions/mika#862
branch: engine/862/asserted-unavailability-endturn-guard
origin: docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md (Rule 2 forward-pointer)
related: senara-solutions/mika#863 (Rule 1 quoted-resource pre-fetch — sibling), senara-solutions/mika#864 (verdict-line ghosting — sibling), senara-solutions/mika#870 (callback-terminal-action — adjacent registry pattern)
---

# engine: asserted-unavailability EndTurn guard for tools claimed unavailable but in active registry

## Overview

mika#862 is the structural counterpart to Rule 2 of the gate-evasion compound doc (`docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`). The compound doc names the failure pattern: under cognitive load, agents (mika-arch in particular) generate "I don't have access to X" / "X is not callable here" phrases without attempting the call to X. Recurrence-1 was mika#654 (`gh_read` falsely claimed "skill-scoped, not callable" three times before required-tools-gate caught it on turn 8). Recurrence-2 was mika#788 (sufficiency hallucination, different shape but same gate).

Prompt-level enforcement of "attempt the call before rationalizing skip" doesn't bind under load (compound doc Rule 3, trace `03d3ec38-0839-47b6-9226-111b38d8b52b`). This plan adds a structural EndTurn post-condition guard that detects the asserted-unavailability phrase pattern, reconciles against the agent's *enabled* tool registry, and re-prompts the agent to either attempt the call or surface the actual failure mode.

The guard sits in the same `INTENT_GUARDS` registry pattern as the existing webhook/resume guards (`agent.rs:3989-4041`) and the just-groomed `callback_terminal_action` guard (mika#870). Naming-symmetric with siblings #863/#864.

## Problem Frame

### Observed failure (from compound doc + grooming today)

In tonight's mika#871 grooming, mika-arch's pass-1 response opened with: *"gh_read is not callable in this CLI context — skill-scoped tool not exposed here. Disclosing this gap explicitly per transparency rule."*

The disclosure is correct in this specific session (CLI-mode mika-arch sessions don't expose `gh_read`), but the failure mode the compound doc warns against is the agent generating that disclosure *without* having attempted the call first. mika#654 had the same shape but was a fabrication: `gh_read` was in the registry, the assertion was wrong, three turns wasted before the existing `required-tools` gate caught it. That gate fires on missing-but-required-tool calls; it does NOT fire on "agent claimed unavailability without attempting." The asserted-unavailability guard fills that hole.

### Root cause (per the compound doc)

The compound doc's Rule 2 ("Always attempt the call before rationalizing the skip") is prompt-level. Prompts don't bind under cognitive load — agents rationalize around them when token pressure or time pressure rises. The structural counterpart is an EndTurn post-condition: if the agent ends a turn with text asserting tool unavailability AND that tool is in the agent's enabled registry AND no successful call to it landed in the turn, reject the EndTurn and re-prompt with a corrective system message naming the violation.

### Existing guard chain reference

Per `crates/mika-agent/CLAUDE.md` (Post-Conditions / EndTurn Chain) and the #870/#871 explore mappings, the chain at `agent.rs:955-1333` has seven sequential guards plus an early-accept. Guard #6 (`Intent-precondition registry` at `agent.rs:3989-4041`) is the registry-driven family the new guard joins. Existing entries: `webhook_ready_label_dispatch`, `webhook_zero_tools`, `resume_reconcile`. The just-groomed mika#870 adds `callback_terminal_action`. This plan adds `asserted_unavailability` as the fifth registry entry.

## Requirements Trace

- **R1.** New `IntentPrecondition` entry `asserted_unavailability` registered in `INTENT_GUARDS` at `crates/mika-agent/src/agent.rs:3989`. **Trigger predicate:** assistant text matches one of the five compound-doc-cited phrase patterns (case-insensitive, all using **named capture group `(?P<tool>...)` uniformly** so extraction is `captures["tool"]` regardless of pattern shape — F2 resolution):
  - `(?i)i (don'?t|do not) have access to (?P<tool>[a-z_][a-z0-9_]*)`
  - `(?i)(?P<tool>[a-z_][a-z0-9_]*) is not (available|callable|accessible)`
  - `(?i)(?P<tool>[a-z_][a-z0-9_]*) isn'?t (available|callable|accessible)`
  - `(?i)(?P<tool>[a-z_][a-z0-9_]*) is skill-scoped` (literal phrase from mika#654)
  - `(?i)cannot call (?P<tool>[a-z_][a-z0-9_]*)`
  Each pattern captures `<tool>` as a snake-case identifier; multi-word references like "the gh_read tool" are normalized by stripping common preamble words. **Satisfied predicate:** `<tool>` does NOT resolve to an entry in the agent's *turn-start enabled-tool set* (i.e., the assertion is structurally true) OR `<tool>` resolves AND the turn's tool-call summaries contain a successful call to `<tool>` (i.e., the agent backed up the assertion with a real attempt). On both branches, satisfaction = no violation. **Self-limiting two-layer false-positive filter (F5 note):** the snake-case identifier constraint filters most natural-language matches; the enabled-set lookup filters the rest. A sentence like "the service is not available in this context" extracts `service`, which is not in the registry → satisfied returns true → no violation fires. The guard fires only when the extracted token IS in the enabled set AND no successful call to it landed.
- **R2.** **Enabled-tool set source — TURN-START SNAPSHOT, not guard-fire-time recomputation (F1 resolution).** The "enabled tool set" is the `name` fields of the tool array passed to the LLM at turn-start (already computed at `agent.rs:~2700` after `apply_agent_tool_visibility` denylist + `apply_identity_allowlist` + `skill_overrides.enabled=0` evictions). The plan adds an `enabled_tool_names: HashSet<String>` field to the post-condition context struct, populated at the same site that builds the LLM tool array. Guard's satisfied predicate consumes the snapshot. **Why snapshot, not recomputation:** if any of the three filters are context-sensitive (e.g., `apply_agent_tool_visibility` varies per skill invocation state), guard-fire-time recomputation could yield a *different* set than what the LLM actually saw — false-positive (tool LLM couldn't see triggers guard) or false-negative (genuinely absent tool fails to satisfy via the "not in registry" branch). The guard verifies *what the LLM was offered*, not *what the engine would offer now*. Per review-guide.md §6 orthogonality: guard's satisfaction check must be a pure function of the turn's observable state.
- **R3.** **On violation:** reject EndTurn, inject corrective system message via the existing rejection path, re-enter the loop. Re-fire prevented by `intent_guard_retries: HashSet<&'static str>` at `agent.rs:803`. Pattern matches all four existing/concurrent registry entries (`webhook_zero_tools`, `webhook_ready_label_dispatch`, `resume_reconcile`, `callback_terminal_action`).
- **R4.** **Corrective system message** (with compound-doc path citation per F6):
  ```
  [Your response was rejected because you claimed <tool> is unavailable,
   but <tool> is in your active tool registry for this session. Attempt the
   call directly. If it fails (auth, rate limit, network, permission), surface
   the actual failure — that is a real signal. "Not callable" without an
   attempt is a fabrication. See docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md
   Rule 2.]
  ```
  The `<tool>` substitution uses `captures["tool"]` (named group, uniform across all five patterns). Compound-doc path citation matches the established pattern from #870's corrective message (which cites `feedback_prompt_enforcement_fragile.md`).
- **R5.** **Eval coverage** in `crates/mika-agent/tests/eval/grounding_regressions/`. Two scenarios:
  - **`asserted_unavailability_caught.rs`** — fixture where `MockLlmProvider` first turn emits `"gh_read is not callable in this CLI context"` with `gh_read` registered as enabled. Assert: guard fires once, corrective message injected, turn 2 either calls `gh_read` (success path) or surfaces an actual error (e.g., "auth failed: ..."). The frozen pre-fix fixture `fixtures/asserted_unavailability_caught_pre_fix.json` reproduces the mika#654 trace shape.
  - **`asserted_unavailability_genuine.rs`** — fixture where the agent says `"gh_read is not callable in this CLI context"` and `gh_read` is in `MIKA_ARCH_DISABLED_TOOLS`. Assert: guard does NOT fire (satisfied predicate's "tool not in enabled registry" branch is true). Verifies the F2-equivalent edge case the issue body calls out as load-bearing.
- **R6.** Compound doc update: append a new section to `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` Rule 2 noting that mika#862's `asserted_unavailability` guard is the structural counterpart now in place. Reference the trace IDs from mika#654 + mika#788 as the recurrence evidence the guard closes on.
- **R7.** No new DB columns or schema migrations. The required state (assistant text, enabled tool registry, current-turn tool-call summaries) is already available to `IntentPrecondition::satisfied`.

## Proposed Fix

### Primary: engine guard

**Where:** `crates/mika-agent/src/agent.rs:3989` — append a fifth `IntentPrecondition` entry after the just-groomed `callback_terminal_action` (mika#870).

```rust
// Pseudocode aligned with existing entries (e.g., webhook_zero_tools at agent.rs:4017-4027)
// #862 — asserted-unavailability guard. Rule 2 of the gate-evasion compound doc.
// Catches "X is not callable" / "I don't have access to X" / "X is skill-scoped"
// when X is in fact in the agent's enabled tool registry and no call was attempted.
IntentPrecondition {
    label: "asserted_unavailability",
    trigger: detect_asserted_unavailability,
    satisfied: asserted_unavailability_satisfied,
    correction_message: "[Your response was rejected because you claimed a tool is unavailable, \
         but the tool is in your active tool registry for this session. \
         Attempt the call directly. If it fails (auth, rate limit, network, permission), \
         surface the actual failure mode — that is a real signal. 'Not callable' \
         without an attempt is a fabrication. See docs/solutions/best-practices/\
         required-tools-gate-evasion-patterns-2026-04-28.md Rule 2.]",
}
```

**Trigger predicate** (`detect_asserted_unavailability`): runs the five compound regexes against assistant text, returns `Some(tool_name)` on first match. Tool name capture group is normalized to `[a-z_][a-z0-9_]*` (strips backticks, "the ... tool" preamble).

**Satisfied predicate** (`asserted_unavailability_satisfied`): takes `(tool_name: &str, enabled_tools: &HashSet<String>, summaries: &[ToolCallSummary])`. Returns `true` iff `!enabled_tools.contains(tool_name)` (assertion structurally true) OR `summaries.iter().any(|s| s.name == tool_name && s.success)` (assertion backed by a real attempt). The `enabled_tools` set is threaded through the post-condition chain from the existing computation site at `agent.rs:~2700` (where `apply_agent_tool_visibility` filters the LLM's tool array).

**On violation:** existing rejection path injects corrective message, loop re-enters. Re-fire prevented by `intent_guard_retries`.

The five-pattern regex set is intentionally bounded to snake_case identifier captures to avoid false-positives on:
- Code examples in markdown (where tool names are inline-coded; the backtick-stripping normalization handles the common case)
- General "X is not available" prose where X is a concept, not a tool (the snake_case constraint excludes natural-language words like "the API")
- Negated assertions about disabled functionality ("X is not enabled" — different verb, not matched)

### Secondary: corrective message refinement

The R4 corrective message captures the violation reason and points the agent at the compound doc. The `<tool>` substitution is implementation-time — in the `correction_message` const, the `<tool>` placeholder is replaced via `format!` when the violation fires. (Per the existing `webhook_ready_label_dispatch` pattern at `agent.rs:4008-4014`, correction messages can be templated with violation-specific context.)

### Tests

**File:** `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_caught.rs` and `asserted_unavailability_genuine.rs` (new), modelled on the existing `grounding_regressions/` scaffold per `mika/CLAUDE.md` Eval — Grounding Regressions (#741) section.

Scenario 1 — **`asserted_unavailability_caught`:**
- `EvalHarness` configures `gh_read` as enabled (no denylist).
- `MockLlmProvider` turn 1: emits `"gh_read is not callable in this CLI context"`, no tool calls, EndTurn.
- Assert: `intent_guard_retries` contains `"asserted_unavailability"`; corrective system message injected; turn 2 emitted (loop did not exit).
- Turn 2: `MockLlmProvider` returns `[gh_read({"op":"issue_view","target":"1","repo":"x/y"}), text("Issue title: ...")]`. Assert: loop exits cleanly; `messages` has assistant rows.
- Frozen pre-fix fixture `fixtures/asserted_unavailability_caught_pre_fix.json` reproduces the mika#654 three-turn fabrication trace.

Scenario 2 — **`asserted_unavailability_genuine`:**
- `EvalHarness` configures `gh_read` in `MIKA_ARCH_DISABLED_TOOLS` (denylisted for mika-arch).
- `MockLlmProvider` turn 1: emits `"gh_read is not callable in this CLI context"`, no tool calls, EndTurn.
- Assert: `intent_guard_retries` does NOT contain `"asserted_unavailability"` (satisfied returned true); loop exits cleanly after turn 1; one assistant message persisted.
- Verifies the false-positive guard works: genuinely disabled tools don't trigger.

Tag vocabulary additions per the `grounding_regressions/README.md` `grounding:*` namespace: `unavailability-asserted-without-attempt` (failure tag for pre-fix fixture), `unavailability-asserted-genuine` (correct-behavior tag).

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Add `detect_asserted_unavailability` and `asserted_unavailability_satisfied` predicate fns; append new `IntentPrecondition` entry to `INTENT_GUARDS` at line ~3989 (after `callback_terminal_action` from mika#870 if that lands first; otherwise at the end of the existing array); thread `enabled_tools: &HashSet<String>` into the post-condition closure context at the existing tool-array-build site (~line 2700). |
| `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_caught.rs` | New file — Scenario 1 (caught fabrication). |
| `crates/mika-agent/tests/eval/grounding_regressions/asserted_unavailability_genuine.rs` | New file — Scenario 2 (false-positive avoided). |
| `crates/mika-agent/tests/eval/grounding_regressions/fixtures/asserted_unavailability_caught_pre_fix.json` | New fixture — frozen pre-fix trace shape from mika#654. |
| `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` | Register new scenarios per existing pattern. |
| `crates/mika-agent/tests/eval/grounding_regressions/README.md` | Add the two new tag entries to the tag vocabulary section; add the two scenarios to the capability matrix. |
| `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` | Append note to Rule 2 referencing mika#862 as the structural counterpart now in place; cite mika#654 + mika#788 trace IDs as recurrence evidence. |
| `CHANGELOG.md` | Add entry under "Fixed" — "Engine now rejects assistant turns that claim a tool is unavailable when the tool is in the agent's enabled registry. Closes #862." |

No schema changes. No new dependencies. No new env vars.

## Verification

### Unit / integration

```bash
cd /data/workspace/mika-platform/.claude/worktrees/engine-862-asserted-unavailability-endturn-guard/mika
cargo test -p mika-agent --test eval grounding_regressions::asserted_unavailability
cargo test -p mika-agent  # full suite
cargo clippy -- -D warnings
cargo fmt --check
```

### Manual reproduction (post-merge)

The compound doc trace `03d3ec38-0839-47b6-9226-111b38d8b52b` is the pre-fix fingerprint. After deploy:

1. Restart mika-server.
2. Run a mika-arch CLI ask that historically triggered the fabrication (e.g., a complex multi-source brief with embedded GitHub references that should be cross-checked via `gh_read`).
3. Inspect the resulting session in `~/.mika/data/mika.db`:
   ```sql
   SELECT trace_id FROM messages
     WHERE agent_id = 'mika-arch' AND role = 'assistant'
       AND content LIKE '%not callable%'
     ORDER BY created_at DESC LIMIT 1;
   ```
4. For that `trace_id`: confirm the next assistant turn either contains a `gh_read` tool call OR surfaces a structural error (auth/network/permission). The pre-fix shape — three turns of "X is skill-scoped" with no attempt — must not appear.

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Regex false-positives on code examples or general prose. | Snake-case capture group constraint (`[a-z_][a-z0-9_]*`) eliminates natural-language matches. Backtick stripping normalization is implementation-time. Eval scenario 1 covers the precise pattern from mika#654; if novel false-positives surface, surface as a regex tuning follow-up. |
| Tool-name capture wrong for multi-token references ("the gh_read tool"). | Normalization strips common preamble words ("the", "this", "that"). If the agent uses an uncommon preamble, the guard misses; better than false-positive. |
| Enabled-tool-registry computation cost added to post-condition chain. | The set is already computed at `agent.rs:~2700` for the LLM tool array. Threading the existing computation through the closure is zero-cost (no recomputation). |
| Guard interferes with mika#863 (quoted-resource pre-fetch) or mika#864 (verdict-line) once those land. | Each guard has a distinct trigger predicate and label. They don't share state. Registry order: this one is fifth (after callback_terminal_action), placement is additive. |
| Corrective message wording drifts from compound doc Rule 2 over time. | Single source of truth: the corrective message cites the doc path. Reviewer enforces drift detection at PR-review time. |
| `<tool>` substitution in correction message exposes a tool name the user shouldn't see (information leakage). | Tool names are operational, not secret. The agent can already mention them freely in normal prose. No new exposure surface. |

## Out of Scope

- **mika#863 (Rule 1 quoted-resource pre-fetch).** Sibling guard, separate plan. Different trigger predicate (citation pattern in text), different satisfied check (was the resource fetched?).
- **mika#864 (verdict-line ghosting).** Sibling guard, separate plan. Different surface (skill-spec output discipline).
- **Pre-condition guards** (gating tool *availability* at prompt-build time rather than at EndTurn). The issue body explicitly cites these as a more invasive design surface, deferred.
- **Shared helper extraction across the four guards (#862/#863/#864/#870).** Per mika#870's plan: revisit when the second EndTurn-family guard ships. This plan does NOT introduce a shared helper.
- **Closure-based correction-message migration** (F3 sentinel). Currently `correction_message: &'static str` works because only `<tool>` substitution is needed and can be handled inline at the violation site. **Migration trigger:** when the second guard with dynamic-message-substitution requirements lands (candidates: #863 resource-name substitution, #864 verdict-line excerpt), replace `correction_message: &'static str` with `correction_message: fn(&PostConditionContext) -> String` in the `IntentPrecondition` struct. Filed as a sentinel here so the migration path doesn't evaporate; not a YAGNI design now.
- **Pattern-addition protocol (F4 codification).** Additional regex patterns for `asserted_unavailability` are added directly to `agent.rs`, but each addition MUST be preceded by a compound-doc Rule 2 update with the observed fabrication phrase + trace ID citation. Prevents silent pattern accumulation without institutional record. Documented here as the protocol for future maintainers.
- **Backfilling pre-existing fabrication-shaped sessions in the audit log.** The guard is forward-looking; historical fabrications stay as-is.

## Open Questions for mika-arch

1. **Regex pattern set completeness.** The five patterns are sourced from the compound doc Rule 2. mika-arch may have observed additional shapes ("X is unavailable in this context", "I lack permission to call X", etc.) that should be added. The architecture is open to adding patterns; my proposal is to ship the five compound-doc-cited patterns and add more as future fabrications surface.
2. **`<tool>` substitution implementation timing.** I treated it as implementation-time (`format!` in the violation arm). Alternative: define `correction_message` as a function returning `String` rather than `&'static str` — would let the registry hold richer template logic. May be a refactor worth doing now while adding the third dynamic-message guard. Defer-to-architect.
3. **Enabled-tool-registry threading.** R2 says "thread the filtered set through the post-condition chain context." The exact threading pattern — adding a field to the existing context struct vs. recomputing at guard-fire-time — is implementation-detail. Architect may prefer one approach.
4. **Test coverage breadth.** Two scenarios (caught, genuine). Worth adding a third (false-positive avoidance for code examples)? My read: YAGNI without observed pressure — the snake-case regex constraint already handles the common cases. Defer-to-architect.

---

## Architect first-pass concerns (resolved in this revision)

This revision applies the six findings from mika-arch's first-pass review (session `ab2c26e8-70b7-4132-a91a-99a5a1a6ebd7`).

### F1 — Turn-start snapshot, not guard-fire-time recomputation (BLOCKING, resolved)

R2 now states the enabled-tool set MUST be a turn-start snapshot of the LLM tool array (via new `enabled_tool_names: HashSet<String>` field on the post-condition context struct, populated at `agent.rs:~2700` where the LLM tool array is built). Guard's satisfied predicate consumes the snapshot. Recomputation at guard-fire-time was the rejected alternative — context-sensitivity in any of the three filters (`apply_agent_tool_visibility`, `apply_identity_allowlist`, `skill_overrides.enabled`) could yield a different set than what the LLM actually saw, producing false-positives or false-negatives. Per review-guide.md §6 orthogonality, the guard's satisfaction check must be a pure function of the turn's observable state.

### F2 — Named capture groups uniformly (BLOCKING, resolved)

All five regex patterns now use `(?P<tool>[a-z_][a-z0-9_]*)` for the tool-name capture. Pattern 1 (`I (don't|do not) have access to ...`) previously had the modal verb in capture group 1 and the tool name in group 2; named captures eliminate the offset-mismatch class entirely. Extraction is `captures["tool"]` uniformly across all five patterns.

### F3 — Closure-based correction-message migration sentinel (sharpening, applied)

Out of Scope section now names the migration trigger: when a second guard with dynamic-substitution requirements lands (#863 resource-name, #864 verdict-line excerpt), replace `correction_message: &'static str` with `correction_message: fn(&PostConditionContext) -> String` in the `IntentPrecondition` struct. Filed sentinel; not designed now per YAGNI.

### F4 — Pattern-addition protocol (sharpening, applied)

Out of Scope section codifies: additional patterns are added directly to `agent.rs`, but each addition MUST be preceded by a compound-doc Rule 2 update with the observed fabrication phrase + trace ID citation. Prevents silent pattern accumulation without institutional record.

### F5 — Self-limiting two-layer false-positive filter documented (sharpening, applied)

R1 now includes the explanatory note: "the snake-case identifier constraint filters most natural-language matches; the enabled-set lookup filters the rest. A sentence like 'the service is not available in this context' extracts `service`, which is not in the registry → satisfied returns true → no violation fires." This documents the safety property for future maintainers without adding a third test scenario.

### F6 — Compound doc path in corrective message (sharpening, applied)

R4's corrective message now includes the literal path `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md Rule 2`. Matches the established pattern from mika#870's corrective message (which cites `feedback_prompt_enforcement_fragile.md`).

---

## Architect verdict

- **First-pass (mika-arch session `ab2c26e8-70b7-4132-a91a-99a5a1a6ebd7`):** ITERATE. Two blockers (F1 turn-start snapshot, F2 named capture groups) + four sharpenings (F3-F6). All resolved in this revision.
- **Second-pass:** pending.
