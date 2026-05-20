---
ticket: mika#852
type: fix
title: "Follow-up to #847: cross-crate marker contract test, 95%-of-cap prompt-size warn gate, stall log counter, Ready-Label disambiguation sentence, predicate comment naming all terminal-rejection cases"
branch: fix/847-followup-cross-crate-marker-prompt-size-gate
created: 2026-05-20
---

# Plan — mika#852 follow-up to #847

Five separable, low-blast-radius improvements to the structural guards landed in #847. The ticket bundles them because each is small and the surface (webhook ready-label dispatch) is one cohesive area.

## 0. Body-premise reconciliation (read before planning)

The ticket body's premise paragraphs were written shortly after #847 merged on 2026-05-09; in the eleven days since, two follow-up PRs (#910 / #1102 — webhook-fallthrough predicate extraction, #1106 — Ready-Label Dispatch carved into its own skill) moved several of the surfaces the ticket cites. The plan below reflects the **current** codebase, not the body. The premise drifts are:

| Body claim | Current state | Material to the AC? |
|------------|---------------|---------------------|
| Marker constant `READY_LABEL_DISPATCH_MARKER` lives in `crates/mika-agent/src/agent.rs` as private const | Marker is in `crates/mika-agent/src/webhook_dispatch.rs:13` as `pub(crate) const`; mika-gateway is the producer, mika-agent the consumer; both crates depend on `mika-common` but **mika-agent does not depend on mika-gateway** | Yes — Fix Path A as worded ("promote the const to `pub` in `mika-gateway::github`, import it in `mika-agent::agent`") is **impossible** under current dep graph. The plan adopts Fix Path B (cross-crate fixture test) and additionally extracts the shared format-prefix constant into `mika-common` so both producer and consumer compile against the same symbol (single source of truth). |
| `self-dev/system_prompt.md` is at 56,674 / 57,344 bytes (98.83% of cap, 670 bytes headroom) | `self-dev/system_prompt.md` is **50,153 bytes**; `[skill] max_prompt_size = 65536`; ceiling `MAX_PROMPT_SIZE_CEILING = 80 * 1024 = 81,920`. Current headroom: 15,383 bytes (76.5% of cap). | No — the structural "100% silent cliff" risk class remains and is what the AC structurally guards against. The "or: cap raised" branch of AC2 has effectively already been taken (cap raised from 57,344 → 65,536); the warning gate at 95% is the unfilled half of the AC. |
| Ready-Label Dispatch section is textually adjacent to Webhook Fallthrough in `self-dev/system_prompt.md`; disambiguation needed because of textual adjacency | Per mika#1106, Ready-Label Dispatch is carved into its own keyword-triggered skill at `skills/bundled/self-dev-webhook-ready-label/system_prompt.md`. Webhook Fallthrough still lives in `skills/bundled/self-dev/system_prompt.md` (§ "Webhook Fallthrough (no keyword-matched handler)", line 115). They are no longer in the same prompt file. | Partly — the textual-adjacency risk is gone (different prompt files load on different turns). The defensive-prose value of an "applies only when label is exactly `ready`" sentence still holds as a tripwire against a future marker-shape change that broadens the trigger surface. Plan applies the sentence to the *new* section location. |
| `ready_label_dispatch_satisfied` is at `crates/mika-agent/src/agent.rs` ~line 3608, satisfied predicate counts attempts not successes, surfaces "the four terminal-rejection cases by name: `global_dispatch_active`, `task_not_dispatchable`, `dispatch_blocked_by`, `dispatch_limit_exceeded`" | Predicate is at `crates/mika-agent/src/agent.rs:5212`. The current `validate_dispatch_readiness` rejection set is **seven** variants per `crates/mika-agent/CLAUDE.md § Dispatch-readiness guard (#525)`: `unauthorized_webhook_dispatch` (#933, check 0), `task_not_dispatchable` (1), `task_active_dispatch` (2), `global_dispatch_active` (3), `dispatch_limit_exceeded` (4), `dispatch_no_grooming_marker` (#919, check 5), `dispatch_blocked_by` (#713, check 6). The four named in the ticket are a strict subset. | Partly — the AC text says "naming the four terminal-rejection cases." Plan documents **all seven** (naming all four ticket-listed cases plus the three added by subsequent PRs) and explains *why* the count expanded — a comment that lists only the four would re-introduce the staleness class the comment is meant to prevent. |

These four drifts inform but do not derail the AC. Phase 2.5 will surface them as a divergence list; if Vincent prefers, the ticket body can be edited to match, or the architect can arbitrate at first-pass.

## 1. AC1 — Cross-crate marker contract enforcement

**Goal:** make a future rename of `mika_gateway::github::format_event_text`'s `[GitHub] Issue labeled ready on ` prefix produce a **compile-time** or **CI-time** signal rather than a silent runtime dispatch hole.

**Constraint:** `mika-agent` cannot depend on `mika-gateway` (would create a reverse dep direction — the agent crate is by design unaware of the gateway). Both crates already depend on `mika-common`.

**Chosen path: shared-constant + cross-crate fixture test (combines Fix Path A spirit with Fix Path B's mechanism).**

### 1.1 Extract the marker constant to `mika-common`

Create `crates/mika-common/src/github_event_format.rs` with:

```rust
//! Shared format-prefix constants for GitHub webhook event text emitted by
//! `mika-gateway::github::format_event_text` and consumed by
//! `mika-agent::webhook_dispatch`. Single source of truth — drift between
//! producer and consumer is a contract violation; this module enforces
//! compile-time coupling.

/// Prefix emitted by `format_event_text` for `issues.labeled` events
/// where the label name is `ready`. The trailing space is significant —
/// the consumer parses `<repo>#<n>` immediately after the prefix.
pub const READY_LABEL_DISPATCH_MARKER: &str = "[GitHub] Issue labeled ready on ";
```

Wire from `crates/mika-common/src/lib.rs`:
```rust
pub mod github_event_format;
```

### 1.2 Switch `mika-agent` to import the shared constant

`crates/mika-agent/src/webhook_dispatch.rs:13` — delete the private const, re-export via `pub(crate) use mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER;` (or update the two existing consumer sites in this file + `agent.rs:5189` to import directly from `mika_common::github_event_format`). Existing call sites (lines 36, 53 in webhook_dispatch.rs; line 5253 in agent.rs) compile unchanged because the symbol name is preserved.

### 1.3 Switch `mika-gateway` to emit using the shared constant

`crates/mika-gateway/src/github.rs::format_event_text` — locate the literal `"[GitHub] Issue labeled ready on "` in the `issues.labeled` branch and replace with `mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER`. The producer now compiles against the same symbol the consumer reads. Renaming the constant is a one-symbol change that both sides see; renaming the producer's output string without touching the const is now impossible because the string literal has been removed.

### 1.4 Add a cross-crate fixture-shape test (defense-in-depth)

`crates/mika-gateway/src/github.rs` — extend the existing `#[cfg(test)] mod tests` (the file already has `test_format_event_text_issue_opened`, `..._pr_opened`, etc. at lines 1361–1601). Add a new test:

```rust
#[test]
fn test_format_event_text_ready_label_marker_contract() {
    // Mirror the [GitHub] Issue labeled ready on <repo>#<n> shape consumed
    // by mika-agent::webhook_dispatch::is_ready_label_dispatch_marker.
    // If this test fails after a format_event_text edit, the consumer-side
    // guard would silently stop matching — break the build instead.
    use mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER;
    let event = build_issues_labeled_event("ready", "senara-solutions/mika", 933, "title");
    let text = format_event_text("issues", &event);
    assert!(
        text.starts_with(READY_LABEL_DISPATCH_MARKER),
        "format_event_text drifted from READY_LABEL_DISPATCH_MARKER: \
         expected prefix {:?}, got {:?}",
        READY_LABEL_DISPATCH_MARKER,
        text
    );
}
```

The fixture-event builder pattern mirrors existing tests in the file (`build_issues_labeled_event` may need to be added if no existing fixture covers the `issues.labeled` branch). The test lives in `mika-gateway` because the producer is the gateway; it imports the shared constant from `mika-common`, so renaming the constant moves both producer and consumer at once.

### 1.5 Acceptance gate for AC1

- `grep -r '"\[GitHub\] Issue labeled ready on "' crates/ --include='*.rs'` returns **zero** literal occurrences (the only reference is the const declaration in `mika-common`).
- `cargo test -p mika-gateway test_format_event_text_ready_label_marker_contract` passes.
- Renaming `READY_LABEL_DISPATCH_MARKER` to e.g. `READY_LABEL_PREFIX` produces compile errors at all three crate sites (verified manually during dev, not part of CI).

## 2. AC2 — 95%-of-cap prompt-size warn gate, 100% fail (extends #828)

**Current behavior:** `scan_skills_dir()` hard-skips skills whose prompt exceeds `max_prompt_size` (or the per-skill default → ceiling). The skipped skills appear in `scan.skipped` and the existing test `bundled_skills_load_without_oversized_prompts` at `crates/mika-agent/tests/bundled_skills_load.rs` panics on any non-empty `scan.skipped`. That is the 100% fail gate AC2 references; the 95% warn gate does not exist yet.

**Where to add the warn gate:**

Extend `crates/mika-agent/tests/bundled_skills_load.rs`. Add a second test function that walks the same `skills/bundled/` tree and, for each successfully loaded skill, computes:

```rust
let cap = skill_manifest.skill.max_prompt_size
    .unwrap_or(DEFAULT_MAX_PROMPT_SIZE)
    .min(MAX_PROMPT_SIZE_CEILING);
let actual = fs::read(prompt_path)?.len() as u64;
let ratio = actual as f64 / cap as f64;
if ratio >= 0.95 {
    near_cap.push((skill_name, actual, cap, ratio));
}
```

If `!near_cap.is_empty()`, panic with a structured report: skill name, actual bytes, cap, percentage. The test is failure-when-warn — CI treats "approaching cap" as a hard signal so the operator addresses it before the next prompt edit pushes over.

**Why a test, not a warning:** the existing `bundled_skills_load_without_oversized_prompts` uses `cargo test` panics as the CI signal — no logging surface. Same pattern keeps the gate uniform.

**Constants to use:** `DEFAULT_MAX_PROMPT_SIZE` and `MAX_PROMPT_SIZE_CEILING` live in `crates/mika-agent/src/skills/index.rs` (private). Either re-export them as `pub(crate)` for test access, or inline the literal `81920` ceiling in the test with a code comment pointing at the source. **Pick re-export** — keeps the single source of truth.

**Remediation guidance in the panic message:** the warn-gate panic should suggest the same fix paths as the existing fail-gate test: (a) raise `max_prompt_size` in the skill's `skill.toml` toward the ceiling (80 KB), (b) trim the prompt, (c) shard via `[dependencies]` if structurally appropriate.

**On the "cap is a magic number" sub-question in the ticket body:** the ticket asks whether raising `max_prompt_size` is preferable to tightening the gate. Looking at current state: `self-dev/skill.toml` declares `max_prompt_size = 65536`, ceiling is `81_920`. The current 50,153-byte prompt sits at 76.5% of the declared cap, 61.2% of the ceiling. The cap is already a few KB below ceiling, allowing growth room without being on a cliff. Reaching 95% of `65536` is 62,259 bytes (12 KB growth from now) — still well under the ceiling. The 95% gate is meaningful at this cap. **No cap change in this PR.** If a future prompt edit hits 95%, the operator's first option is to raise the cap one notch (e.g., 65536 → 73728), which the warn gate explicitly hints at in its panic message.

### Acceptance gate for AC2

- New test `bundled_skills_approaching_max_prompt_size_warns` in `crates/mika-agent/tests/bundled_skills_load.rs`.
- Test panics with a structured report when any bundled skill's prompt ≥ 95% of its effective cap.
- Test currently passes (no bundled skill above 95% today; closest is self-dev at 76.5%).
- Manual verification: temporarily reduce `max_prompt_size` in a skill's `skill.toml` to a value 5% above the prompt size, observe test failure, revert.

## 3. AC3 — `ready_label_dispatch_stall_total` log event on stall path

**Site:** `crates/mika-agent/src/agent.rs:1762-1784`. Currently emits one `error!(... "ready_label_dispatch_stalled — operator notification fired")` per stall, plus a `send_message` to the operator.

**Mika's metric convention:** no `metrics` crate in the dependency graph (verified — `grep -r 'counter!\|metrics::' crates/` returns nothing relevant). Observability is per-event structured logging via `tracing` (see `crates/mika-agent/CLAUDE.md § Observability — Log Sinks`). The existing `error!` event already serves as a per-stall counter — operators can `grep ready_label_dispatch_stalled | wc -l` to get the total. The ticket says "one line" for a counter and "a single `tracing` counter"; this maps to a dedicated structured event name with a counter-friendly suffix.

**Plan:** Rename the event from `ready_label_dispatch_stalled — operator notification fired` to a counter-friendly form by adding a sibling line with a stable event name:

```rust
error!(
    trace_id = %tool_ctx.trace_id,
    location = %location,
    label = mode.label(),
    "ready_label_dispatch_stall_total"   // stable, grep-friendly counter event
);
error!(
    trace_id = %tool_ctx.trace_id,
    location = %location,
    label = mode.label(),
    "ready_label_dispatch_stalled — operator notification fired"  // existing human-readable line, preserved
);
```

The two-line shape preserves existing log readers (anyone tailing `ready_label_dispatch_stalled` continues to work) while adding a `_total`-suffixed event for counting. Operator runbook for the future "do we need to debounce?" question (per the ticket): `jq 'select(.message == "ready_label_dispatch_stall_total") | .timestamp' < $MIKA_SERVER_LOG_FILE` gives a per-stall timestamp stream.

Add a code comment above the new line citing mika#852 so future readers understand the dual-emission is intentional.

### Acceptance gate for AC3

- `grep -n 'ready_label_dispatch_stall_total' crates/mika-agent/src/agent.rs` returns at least one match in the stall path.
- Manual trace: trigger the stall path (e.g., via the existing test in `agent.rs:7707+`), confirm both event names appear in the captured log.

## 4. AC4 — Disambiguation sentence on Ready-Label Dispatch section

**Section location post-#1106:** `skills/bundled/self-dev-webhook-ready-label/system_prompt.md`. The current opening (lines 1–5):

```
### Ready-Label Dispatch (MANDATORY — do not skip, do not defer)

When the message starts with `[GitHub] Issue labeled ready on <repo>#<n>`, the operator has set the `ready` label on the ticket — the canonical positive-consent signal for autonomous dispatch.

> **The engine enforces this sequence via the `webhook_ready_label_dispatch` intent-precondition guard ...
```

**Plan:** Insert a one-sentence scope clarification immediately after the opening paragraph and before the engine-enforces callout. Proposed text:

```
**Scope:** Applies **only** when the label is exactly `ready`. For any other label (`bug`, `p1-important`, `needs-triage`, etc.), see Webhook Fallthrough in the `self-dev` prompt — those labels are not dispatch consent and must not call `run_claude_pilot`.
```

Why this still has value despite the file separation introduced by #1106: the dedicated skill activates on a *keyword* match (per `self-dev-webhook-ready-label/skill.toml`). If a future format-string change ever broadens the trigger surface (e.g., a new label-shape webhook arrives via a different marker), the prompt-level "applies only when label is exactly `ready`" sentence is the second-line defense against false-positive dispatch. The engine-level `is_ready_label_dispatch_marker` predicate is the first line; this sentence is defense-in-depth on the LLM's interpretation layer.

### Acceptance gate for AC4

- `skills/bundled/self-dev-webhook-ready-label/system_prompt.md` contains the literal phrase `Applies **only** when the label is exactly` (or paraphrase carrying the same scope claim) in the section opening.
- `bundled_skills_load` test still passes (the additional ~250 bytes is well below cap).

## 5. AC5 — Comment-harden the `ready_label_dispatch_satisfied` predicate

**Site:** `crates/mika-agent/src/agent.rs:5212`. The existing doc comment (lines 5200–5211) already explains *why* attempts (not successes) are counted:

> Attempts count regardless of success — terminal failures (global_dispatch_active, task_not_dispatchable, etc.) are structural and not recoverable by re-prompt.

The ticket asks for the four terminal-rejection cases to be named explicitly. The current rejection set in `validate_dispatch_readiness` is **seven** (per `crates/mika-agent/CLAUDE.md § Dispatch-readiness guard (#525)`):

1. `unauthorized_webhook_dispatch` (mika#933, check 0)
2. `task_not_dispatchable` (check 1)
3. `task_active_dispatch` (check 2)
4. `global_dispatch_active` (check 3)
5. `dispatch_limit_exceeded` (check 4)
6. `dispatch_no_grooming_marker` (mika#919, check 5)
7. `dispatch_blocked_by` (mika#713, check 6)

**Plan:** rewrite the doc comment to enumerate **all seven** current variants, not just the four named in the ticket body. The comment becomes:

```rust
/// True when `run_claude_pilot` or `run_claude_pilot_groom` was attempted on this
/// turn (success or failure). The webhook_ready_label_dispatch intent-guard
/// satisfies on **attempts**, not **successes**, because the seven terminal
/// rejections from `validate_dispatch_readiness` are structural and not
/// recoverable by re-prompting the LLM:
///
/// 1. `unauthorized_webhook_dispatch` — non-ready webhook on the prevention surface (mika#933)
/// 2. `task_not_dispatchable` — task is in a terminal state (`blocked`/`completed`/`cancelled`)
/// 3. `task_active_dispatch` — the same task already has an active callback child
/// 4. `global_dispatch_active` — another task of the same dispatch class has an active callback
/// 5. `dispatch_limit_exceeded` — per-turn dispatch counter already at the limit
/// 6. `dispatch_no_grooming_marker` — ungroomed issue rejected at the gate (mika#919)
/// 7. `dispatch_blocked_by` — open GitHub blockers remain (mika#713)
///
/// Re-prompting the LLM after any of these would loop (the LLM cannot dissolve
/// a structural rejection); the operator-notification fires instead.
///
/// History: #907 added an OR-shape (run_claude_pilot || send_message) ...
/// (existing history paragraphs preserved below)
```

**Why all seven and not the four named in the ticket:** the ticket was written when the rejection set was four; today it is seven. A comment listing only the four ticket-named cases would re-introduce the same staleness class the comment is meant to prevent. Future structural rejections added by new dispatch checks will need to be appended; the seven-item list is the canonical snapshot.

### Acceptance gate for AC5

- The doc comment above `ready_label_dispatch_satisfied` at `crates/mika-agent/src/agent.rs:5212` names all seven `validate_dispatch_readiness` rejection variants by their canonical error-string identifier.
- The four ticket-named cases (`global_dispatch_active`, `task_not_dispatchable`, `dispatch_blocked_by`, `dispatch_limit_exceeded`) are all present in the list — superset satisfies the ticket text.

## 6. Sequencing

The five items are independent. Suggested commit order:

1. **AC5 (comment)** — pure documentation, zero behavior change, easiest to verify in isolation.
2. **AC3 (log-event split)** — single-file diff in `agent.rs`, no test signal needed beyond a compile-clean.
3. **AC4 (prompt sentence)** — single-file diff in `self-dev-webhook-ready-label/system_prompt.md`; `bundled_skills_load` test transitively verifies.
4. **AC2 (95% warn gate)** — new test in `bundled_skills_load.rs`; touches no production code.
5. **AC1 (shared marker + cross-crate test)** — touches three crates (`mika-common`, `mika-agent`, `mika-gateway`); largest blast radius, last.

Each item lands as a separate commit on the branch. PR is one bundled PR (per the ticket structure and `feedback_implementation_scope_bundling`).

## 7. Out-of-band notes

- **Item from "Out of scope (file separately)":** the ticket explicitly defers the "process improvement" item (PR template "compound docs consulted" field, mika-arch critique-checklist update). NOT addressed in this PR; if it has not been filed as a separate ticket since 2026-05-09, that filing is itself out-of-scope for this PR but worth surfacing to the operator at PR-open time.
- **Skill-prompt drift watch:** AC2's warn gate at 95% will fire if the self-dev prompt grows from 50,153 bytes (76.5% of 65,536 cap) to ≥ 62,259 bytes (95%). That's 12 KB of growth headroom. Compound docs on "skill prompt growth pressure" do not exist; if AC2 starts firing in the future, the operator can either raise the cap toward the 81,920-byte ceiling or shard the prompt into `[dependencies]`.

## 8. Test plan

- `cargo build -p mika-common -p mika-agent -p mika-gateway` — compiles clean after AC1's symbol move.
- `cargo test -p mika-gateway test_format_event_text_ready_label_marker_contract` — new test passes.
- `cargo test -p mika-agent --test bundled_skills_load` — both existing test and new `bundled_skills_approaching_max_prompt_size_warns` pass.
- `cargo test -p mika-agent --test eval test_ready_label_dispatch_satisfied` — existing tests unchanged (5212 predicate logic untouched, only the comment changed).
- `cargo clippy --all-targets` — clean.

## 9. Acceptance checklist (verbatim from issue body)

- [ ] `READY_LABEL_DISPATCH_MARKER` is shared across crates **OR** a cross-crate fixture test asserts the prefix contract. Renaming the gateway formatter's output produces a compile error or test failure. → **§1, both legs taken (shared const in mika-common + fixture test in mika-gateway).**
- [ ] CI test fails when any bundled skill's `system_prompt.md` exceeds 100% of `max_prompt_size`, warns at 95%. → **§2, new warn test added; 100% gate already exists in `bundled_skills_load_without_oversized_prompts`.**
- [ ] `ready_label_dispatch_stall_total` counter increments alongside the operator notification. → **§3, dedicated structured log event added.**
- [ ] Ready-Label Dispatch section opens with the "applies *only* when label is exactly `ready`" sentence. → **§4, sentence added at new section location (self-dev-webhook-ready-label per mika#1106).**
- [ ] `ready_label_dispatch_satisfied` has a code comment naming the four terminal-rejection cases. → **§5, all seven current cases named (superset).**
