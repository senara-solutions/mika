---
ticket: mika#852
type: fix
title: "Follow-up to #847: cross-crate marker contract test, 95%-of-cap prompt-size warn gate, stall log counter, Ready-Label disambiguation sentence, predicate comment naming all terminal-rejection cases"
branch: fix/847-followup-cross-crate-marker-prompt-size-gate
created: 2026-05-20
base_sha: 498c536a
---

# Plan — mika#852 follow-up to #847

Five separable, low-blast-radius improvements to the structural guards landed in #847. The ticket bundles them because each is small and the surface (webhook ready-label dispatch) is one cohesive area.

## Phase 0 Pin — load-bearing sites at base SHA `498c536a`

Verbatim slices read before planning. Lines below are quoted exactly from the worktree at the recorded SHA. Each subsequent section's edits anchor to these slices.

### Pin-0.1 — `crates/mika-agent/src/webhook_dispatch.rs:1–54` (consumer side)

```rust
//! Shared predicates for webhook dispatch gating (mika#933).
//!
//! Both `agent.rs` (INTENT_GUARDS post-hoc) and `skills/executor.rs`
//! (tool-boundary pre-hoc) consume these predicates. Single source of truth
//! prevents drift between the two guard layers.

/// Marker prefix emitted by `mika_gateway::github::format_event_text` for
/// `issues.labeled` events where the label name is `ready`. The
/// authoritative format string lives at
/// `crates/mika-gateway/src/github.rs` `format_event_text`.
/// Drift between the two locations is a contract violation; the gateway
/// side is the producer, this side is the consumer. See mika#842, mika#910.
pub(crate) const READY_LABEL_DISPATCH_MARKER: &str = "[GitHub] Issue labeled ready on ";

/// True when the message is a `[GitHub]` webhook event in the
/// **Webhook Fallthrough** domain — i.e., a turn that MUST NOT call
/// `run_claude_pilot`. [...]
pub(crate) fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    if !msg.starts_with("[GitHub]") {
        return false;
    }
    if msg.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return false;
    }
    // qa skill territory (Phase 0 prefix surface rows E, F).
    if msg.starts_with("[GitHub] PR ") {
        return false;
    }
    // ci skill territory (Phase 0 prefix surface row G).
    if msg.starts_with("[GitHub] Check suite ") {
        return false;
    }
    // Everything else in [GitHub] domain (rows B, C, D, H) is fallthrough.
    true
}

/// True when the message matches the ready-label dispatch marker prefix.
pub(crate) fn is_ready_label_dispatch_marker(msg: &str) -> bool {
    msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}
```

**Module-level test fixtures already cover `Row A` (ready-label) and `Row B` (non-ready label).** New cross-crate contract test goes in `mika-gateway`, not here.

### Pin-0.2 — `crates/mika-gateway/src/github.rs:299, 320–334` (producer side, `issues.labeled` arm)

```rust
pub fn format_event_text(event_type: &str, event: &GitHubWebhookEvent) -> String {
    [...]
            if action == "labeled"
                && let Some(ref label) = event.label
                && let Some(ref label_name) = label.name
                && !label_name.is_empty()
            {
                return format!(
                    "[GitHub] Issue labeled {label_name} on {repo_name}#{number} — {title}\n{url}"
                );
            }
            // Fallback for labeled without label name: uses generic format below.
```

The literal string starts with `[GitHub] Issue labeled ` (no `ready` token — the label name comes from the event payload). The current consumer-side `READY_LABEL_DISPATCH_MARKER` is `"[GitHub] Issue labeled ready on "` — it's the *interpolated result* for the specific case `label_name = "ready"`, not the format-string template. This means the shared constant cannot be a `format!`-spliceable substring of the producer's template; it has to be checked at the *output* level (i.e., after `format!` runs) by either (a) the existing test that already constructs a `label_name = "ready"` event, or (b) a tiny helper that emits the marker prefix for known-`ready` label-name inputs. **Path (a) is chosen** — augments existing `test_format_event_text_issue_labeled_extracts_label_name` rather than replacing the format string.

### Pin-0.3 — `crates/mika-gateway/src/github.rs:1668–1701` (existing `issues.labeled` test fixture)

```rust
#[test]
fn test_format_event_text_issue_labeled_extracts_label_name() {
    let event = GitHubWebhookEvent {
        action: Some("labeled".to_string()),
        sender: None,
        installation: None,
        check_suite: None,
        issue: Some(GitHubIssue {
            number: Some(841),
            title: Some("Gate dispatch on ready label".to_string()),
            html_url: Some("https://github.com/senara-solutions/mika/issues/841".to_string()),
            body: None,
            assignee: None,
        }),
        pull_request: None,
        comment: None,
        review: None,
        requested_reviewer: None,
        label: Some(GitHubLabel {
            name: Some("ready".to_string()),
        }),
        repository: Some(GitHubRepository {
            full_name: Some("senara-solutions/mika".to_string()),
            html_url: None,
        }),
        before: None,
        after: None,
    };

    let text = format_event_text("issues", &event);
    assert_eq!(
        text,
        "[GitHub] Issue labeled ready on senara-solutions/mika#841 — Gate dispatch on ready label\nhttps://github.com/senara-solutions/mika/issues/841"
    );
}
```

**This test ALREADY constructs the exact fixture the contract test needs.** F2 (architect first-pass) is resolved by adding **one additional assertion** to this test rather than building a new fixture builder. Inline `GitHubWebhookEvent { ... }` struct construction is the codebase convention (mirrored by the other 6 tests in the same `mod tests`). No shared `build_issues_labeled_event` helper exists today; one is not needed for §1.4.

### Pin-0.4 — `crates/mika-agent/src/agent.rs:1757–1784` (stall path)

```rust
                    // #846 + #907 + #1089 — operator notification when the
                    // ready-label dispatch guard fired but run_claude_pilot was
                    // not called after the retry.  Without this the failure is
                    // silent past label-removal: the ready label disappears but
                    // dispatch never happens.
                    if intent_guard_retries.contains("webhook_ready_label_dispatch")
                        && !ready_label_dispatch_satisfied(&all_tool_summaries)
                    {
                        let location = parse_ready_label_location(&user_input_text)
                            .unwrap_or_else(|| "<unknown>".to_string());
                        error!(
                            trace_id = %tool_ctx.trace_id,
                            location = %location,
                            label = mode.label(),
                            "ready_label_dispatch_stalled — operator notification fired"
                        );
                        if let Some(ref sender) = tool_ctx.message_sender {
                            let notification = format!(
                                "Ready-label dispatch stalled on {location}: the `ready` \
                                 label was removed but dispatch (run_claude_pilot) did \
                                 not complete. Investigate trace_id {} in \
                                 /var/log/mika/server.log. To retry, re-add the `ready` \
                                 label.",
                                tool_ctx.trace_id
                            );
                            let _ = sender.send(&notification).await;
                        }
                    }
```

Block structure: `if condition { let location; error!(...); if let Some(sender) { format! + send().await }; }`. AC3 inserts a sibling `error!` line above the existing one (same field shape: `trace_id`, `location`, `label`), preserving the existing message for backward log-reader compatibility.

### Pin-0.5 — `crates/mika-agent/src/agent.rs:5187–5219` (predicate + re-export)

```rust
/// Re-export from `webhook_dispatch` module — single source of truth for the
/// ready-label marker prefix (mika#933).
use crate::webhook_dispatch::{READY_LABEL_DISPATCH_MARKER, is_unauthorized_webhook_dispatch};

/// Triggers when a webhook turn was initiated by the ready-label dispatch
/// marker.  Delegates to `webhook_dispatch::is_ready_label_dispatch_marker`
/// for the single-source-of-truth predicate (mika#933).
fn ready_label_dispatch_trigger(msg: &str) -> bool {
    crate::webhook_dispatch::is_ready_label_dispatch_marker(msg)
}

/// Returns `true` when `run_claude_pilot` was attempted in this turn
/// (success or failure).
///
/// #846, #907, #1089 — Requires `run_claude_pilot` attempted on ready-label
/// webhook turns.  Attempts count regardless of success — terminal failures
/// (global_dispatch_active, task_not_dispatchable, etc.) are structural and
/// not recoverable by re-prompt.  See `INTENT_GUARDS` comment and #846.
///
/// History: #907 added an OR-shape (`run_claude_pilot || send_message`) to
/// accept grooming-rejection notifications.  #996 replaced the rejection path
/// with auto-groom via `run_claude_pilot(dev-groom)`, making `send_message`
/// obsolete for this guard.  #1089 removed `send_message` from the predicate
/// after fabricated `check_task` pre-flights exploited the over-broad match to
/// short-circuit dispatch via a hallucinated escalation that hit NoChannel.
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
    // mika#1173: dev-groom owns its own tool (run_claude_pilot_groom) after the
    // structural revert. Auto-groom path dispatches via the groom tool; both
    // names satisfy the ready-label dispatch contract.
    summaries
        .iter()
        .any(|s| s.name == "run_claude_pilot" || s.name == "run_claude_pilot_groom")
}
```

The `use crate::webhook_dispatch::{READY_LABEL_DISPATCH_MARKER, ...};` at line 5189 is already a re-export — symbol is imported by name into `agent.rs` scope and used at line 5253 (`parse_ready_label_location`). **The "import shape" first-pass uncertainty resolves cleanly: the file already uses the re-export form.** AC1 just relocates the source-of-truth one crate deeper (`webhook_dispatch` → `mika_common`) without changing the local symbol name. Existing call site at agent.rs:5253 keeps working unchanged.

### Pin-0.6 — `crates/mika-agent/src/skills/index.rs:17–22` (size constants)

```rust
/// Default maximum size for system_prompt.md snippets (16 KB).
pub(super) const MAX_PROMPT_SNIPPET_SIZE: u64 = 16 * 1024;

/// Hard ceiling for per-skill `max_prompt_size` override (80 KB).
/// Prevents marketplace skills from loading arbitrarily large prompts.
pub(super) const MAX_PROMPT_SIZE_CEILING: u64 = 80 * 1024;
```

**Important nomenclature correction**: the default constant is `MAX_PROMPT_SNIPPET_SIZE` (16 KB), not `DEFAULT_MAX_PROMPT_SIZE` (the earlier-drafted plan citation was wrong). `MAX_PROMPT_SIZE_CEILING` is 80 KB, as cited. Both are `pub(super)`. AC2 needs to re-export them as `pub(crate)` so the integration test under `tests/` can read them.

### Pin-0.7 — `crates/mika-common/src/lib.rs:1–15` (target module file)

```rust
pub mod agent;
pub mod claude;
pub mod config;
pub mod dotenv;
pub mod embedding;
pub mod github_app;
pub mod home;
pub mod llm;
pub mod logging;
pub mod oauth;
pub mod team;
pub mod telemetry;
pub mod text;
pub mod trace;
pub mod validation;
```

Alphabetical order. AC1 adds `pub mod github_event_format;` between `embedding` and `github_app` (alphabetically after `embedding`).

### Pin-0.8 — crate dependency graph

```
mika-common: no internal deps
mika-gateway: depends on mika-common
mika-agent:   depends on mika-common (does NOT depend on mika-gateway)
```

Confirmed: putting the shared constant in `mika-common` is the only correct direction. Putting it in `mika-gateway` would require `mika-agent` → `mika-gateway` dep, which doesn't exist.

### Pin-0.9 — `skills/bundled/self-dev-webhook-ready-label/system_prompt.md:1–9` (target section)

```markdown
### Ready-Label Dispatch (MANDATORY — do not skip, do not defer)

When the message starts with `[GitHub] Issue labeled ready on <repo>#<n>`, the operator has set the `ready` label on the ticket — the canonical positive-consent signal for autonomous dispatch.

> **The engine enforces this sequence via the `webhook_ready_label_dispatch` intent-precondition guard (mika#846, #907, #1089, #1173).** The guard requires a `run_claude_pilot` attempt (dispatch via dev-pilot for implementation) OR a `run_claude_pilot_groom` attempt (auto-groom via dev-groom). Ending the turn without calling one of these will cause the engine to reject your `EndTurn` once and re-prompt you. The steps below are a structural contract, not advisory prose.
```

AC4 inserts the scope sentence as a third paragraph between the existing second paragraph (opening "When the message...") and the blockquote callout. Current prompt size = `wc -c skills/bundled/self-dev-webhook-ready-label/system_prompt.md` = (verified below at AC4 acceptance gate). Adding ~280 bytes is well under the cap.

## 1. AC1 — Cross-crate marker contract enforcement

**Goal:** make a future rename of `mika_gateway::github::format_event_text`'s `[GitHub] Issue labeled ready on ` output prefix produce a **compile-time** or **CI-time** signal rather than a silent runtime dispatch hole.

**Constraint (Pin-0.8):** `mika-agent` cannot depend on `mika-gateway`. Both depend on `mika-common`.

**Chosen path: shared-constant in `mika-common` + assertion added to existing fixture test in `mika-gateway`.**

### 1.1 Extract the marker constant to `mika-common`

Create `crates/mika-common/src/github_event_format.rs`:

```rust
//! Shared format-prefix constants for GitHub webhook event text emitted by
//! `mika-gateway::github::format_event_text` and consumed by
//! `mika-agent::webhook_dispatch`. Single source of truth — drift between
//! producer and consumer is a contract violation; the cross-crate test in
//! `mika-gateway::github::tests` enforces the contract at CI time.

/// Prefix emitted by `format_event_text` for `issues.labeled` events
/// where the label name is `ready`. The trailing space is significant —
/// the consumer parses `<repo>#<n>` immediately after the prefix.
///
/// Producer: `mika_gateway::github::format_event_text` (`issues.labeled`
/// arm; constructs via `format!`, asserted to match this prefix in test
/// `test_format_event_text_issue_labeled_extracts_label_name`).
///
/// Consumers: `mika_agent::webhook_dispatch` (`is_ready_label_dispatch_marker`,
/// `is_unauthorized_webhook_dispatch`); `mika_agent::agent`
/// (`parse_ready_label_location`).
pub const READY_LABEL_DISPATCH_MARKER: &str = "[GitHub] Issue labeled ready on ";
```

Wire from `crates/mika-common/src/lib.rs` (Pin-0.7) — add one line between `embedding` and `github_app` to preserve alphabetical order:

```rust
pub mod github_event_format;
```

### 1.2 Switch `mika-agent` to import the shared constant

Edit `crates/mika-agent/src/webhook_dispatch.rs:7–13` (Pin-0.1):

```rust
/// Marker prefix emitted by `mika_gateway::github::format_event_text` for
/// `issues.labeled` events where the label name is `ready`. Re-exported
/// from `mika_common::github_event_format` for cross-crate single-source-of-
/// truth coupling. See mika#852.
pub(crate) use mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER;
```

This keeps the local symbol name unchanged — call sites at lines 36, 53, and `agent.rs:5253` (via the re-export at agent.rs:5189) compile without edits. **Re-export over direct-import** because Pin-0.5 confirms `agent.rs` already uses the re-export pattern through `webhook_dispatch`; consistent with file's existing shape.

### 1.3 Switch `mika-gateway` to emit using the shared constant

Edit `crates/mika-gateway/src/github.rs:320–334` (Pin-0.2). The current `format!` template hardcodes `[GitHub] Issue labeled `; the shared constant is `[GitHub] Issue labeled ready on ` (interpolated form for the specific `label_name = "ready"` case). The producer's template can't directly substitute the constant because the constant collapses the `{label_name} on ` interpolation — they're different shapes.

**Approach: assertion-based coupling, not literal-substitution coupling.** The producer's format string stays unchanged. The shared constant lives in `mika-common` as a *contract* — the test in §1.4 asserts the producer's `format!` output starts with the constant when `label_name = "ready"`. Renaming the constant fails the test; renaming the producer's `format!` template fails the test. Either rename in isolation breaks CI.

**Optional defense-in-depth (post-#852 if useful):** decompose the format template to use `format!("{prefix}{repo_name}#{number} — {title}\n{url}", prefix = ...)` where `prefix` is computed via a small helper. Out of scope for #852 — the assertion test alone gives the same CI signal without refactoring the producer template.

### 1.4 Augment the existing fixture test with the contract assertion

Edit `crates/mika-gateway/src/github.rs:1697–1700` (the `assert_eq!` at the end of `test_format_event_text_issue_labeled_extracts_label_name`, Pin-0.3). Replace the single `assert_eq!` with **both** the existing full-string equality (preserves the existing exact-shape assertion) AND the new prefix-contract assertion:

```rust
    let text = format_event_text("issues", &event);

    // Contract: the producer's output for label_name="ready" must start with
    // the canonical READY_LABEL_DISPATCH_MARKER prefix shared via mika-common.
    // Renaming the constant or the producer template breaks this assertion
    // and surfaces the cross-crate drift at CI time (mika#852).
    assert!(
        text.starts_with(mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER),
        "format_event_text drifted from READY_LABEL_DISPATCH_MARKER: \
         expected prefix {:?}, got {:?}",
        mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER,
        text,
    );

    // Existing exact-shape assertion (regression: full output stays stable).
    assert_eq!(
        text,
        "[GitHub] Issue labeled ready on senara-solutions/mika#841 — Gate dispatch on ready label\nhttps://github.com/senara-solutions/mika/issues/841"
    );
```

**No new test, no new fixture builder** (resolves F2). The fixture already constructs the `issues.labeled` event with `label_name = "ready"` (Pin-0.3); adding two new assertions on the same `text` variable is a 6-line edit to the existing test body.

### 1.5 Acceptance gate for AC1

- `grep -rn '"\[GitHub\] Issue labeled ready on "' crates/ --include='*.rs'` returns exactly one occurrence: the constant declaration in `crates/mika-common/src/github_event_format.rs`.
- `cargo test -p mika-gateway test_format_event_text_issue_labeled_extracts_label_name` passes (both assertions).
- `cargo test -p mika-agent --lib webhook_dispatch::tests::` passes (consumer-side tests at webhook_dispatch.rs:62-185 unchanged — they assert against the marker through `is_ready_label_dispatch_marker`/`is_unauthorized_webhook_dispatch`).
- Manual verification: rename the constant to e.g. `READY_LABEL_PREFIX` and observe (a) compile errors at the consumer site (webhook_dispatch.rs re-export) and the agent.rs re-export, and (b) test failure on the producer-side assertion if only the constant is renamed but the producer template isn't.

## 2. AC2 — 95%-of-cap prompt-size warn gate (extends #828)

**Current behavior (Pin-0.6):** `scan_skills_dir()` hard-skips skills whose prompt > `max_prompt_size` (or the per-skill default → ceiling). The existing test `bundled_skills_load_without_oversized_prompts` at `crates/mika-agent/tests/bundled_skills_load.rs:46–80` panics on any non-empty `scan.skipped` — that's the 100% fail gate. The 95% warn gate does not exist yet.

**Where to add the warn gate.** Extend `crates/mika-agent/tests/bundled_skills_load.rs`. Add a second test function `bundled_skills_approaching_max_prompt_size_warns` that walks `skills/bundled/` directly (not via `scan_skills_dir`, which only reports cap violations on the failure side) and, for each prompt file, computes:

```rust
let effective_cap = manifest_max_prompt_size
    .unwrap_or(MAX_PROMPT_SNIPPET_SIZE)
    .min(MAX_PROMPT_SIZE_CEILING);
let actual = fs::metadata(prompt_path)?.len();
let ratio = actual as f64 / effective_cap as f64;
if ratio >= 0.95 {
    near_cap.push((skill_name, actual, effective_cap, ratio));
}
```

If `!near_cap.is_empty()`, panic with a structured report: skill name, actual bytes, cap, percentage, and the same remediation guidance the existing 100% test offers (raise `max_prompt_size` toward 80 KB ceiling, trim, or shard via `[dependencies]`).

**Re-exporting `MAX_PROMPT_SNIPPET_SIZE` and `MAX_PROMPT_SIZE_CEILING`.** Both constants are `pub(super)` at Pin-0.6 (visible only within `crates/mika-agent/src/skills/`). The integration test under `crates/mika-agent/tests/` cannot read `pub(super)` symbols — they need to be `pub(crate)`. Bump both to `pub(crate)`. Manifest parsing: read `skill.toml` via `toml::from_str::<SkillManifestRaw>` (or whatever the existing manifest deserialization shape is — same path `scan_skills_dir` uses). Avoid duplicating manifest parsing logic; if the field is not trivially accessible, the test reads `manifest.max_prompt_size: Option<u64>` from a shared `SkillManifest` struct already exposed via the skills module.

**Implementation note on shared parser surface.** If `SkillManifest::max_prompt_size` (or equivalent) is currently a private field on a `pub(super)` struct, expose the parser at a narrow surface (e.g., a `pub(crate) fn parse_skill_manifest(path: &Path) -> Result<SkillManifest>` in `skills::index` mod) rather than re-implementing the parse logic in the test. The test should read the same manifest representation production reads — same single-source-of-truth principle as AC1.

**Panic message shape** (proposed):

```
Bundled-skill prompt approaching cap (≥95% of effective max_prompt_size):

  - self-dev: 62500 bytes / 65536 cap (95.4%, MAX_PROMPT_SNIPPET_SIZE/MAX_PROMPT_SIZE_CEILING enforced)

Remediation options:
  - Raise max_prompt_size in the skill's skill.toml toward the 80 KB ceiling.
  - Trim the prompt — see docs/skill-prompt-trimming-checklist.md if it exists.
  - Shard the prompt via [dependencies] (extract a sibling skill).

This warn-gate (mika#852) defends against silent skill-skip at the 100% cliff
(see bundled_skills_load_without_oversized_prompts for the failure case).
```

### Acceptance gate for AC2

- New test `bundled_skills_approaching_max_prompt_size_warns` lives in `crates/mika-agent/tests/bundled_skills_load.rs`.
- Test panics with a structured report when any bundled skill's prompt ≥ 95% of its effective cap.
- Test currently passes (no bundled skill above 95% today; closest is `self-dev/system_prompt.md` at 50,153 bytes / 65,536 cap = 76.5%).
- Manual verification: temporarily reduce `max_prompt_size` in `self-dev/skill.toml` from `65536` to a value 5% above 50,153 bytes (e.g., `52681`, which makes 50,153 = 95.2% of the new cap), observe test panic with structured remediation, revert.

**On the "cap is a magic number" sub-question.** Plan position: 95% gate at the current cap (65,536) is meaningful — 95% = 62,259 bytes, 12 KB headroom from now. No cap change in this PR. If a future prompt edit pushes 95%, the operator's first move is to raise the cap one notch (65536 → 73728), and the warn-gate panic message explicitly suggests that. The "or: cap raised" branch of the ticket AC has effectively already been taken (cap raised 57344 → 65536 since #847); the warning gate is the unfilled half.

## 3. AC3 — `ready_label_dispatch_stall_total` log event on stall path

**Site:** `crates/mika-agent/src/agent.rs:1762–1784` (Pin-0.4). Currently emits one `error!(... "ready_label_dispatch_stalled — operator notification fired")` per stall, plus a `send_message` to the operator.

**Mika's metric convention:** no `metrics` crate in the dependency graph (verified — `grep -rn 'counter!\|metrics::' crates/` returns no relevant matches). Observability is per-event structured logging via `tracing` (per `crates/mika-agent/CLAUDE.md § Observability — Log Sinks`). The existing `error!` event already serves as a per-stall counter — operators can `grep ready_label_dispatch_stalled | wc -l` for total. The ticket asks for "a single `tracing` counter"; this maps to a dedicated structured event name with a counter-friendly suffix.

**Plan:** Add a sibling `error!` line above the existing one. Both lines carry identical field shape (`trace_id`, `location`, `label`); the new line carries the stable, grep-friendly counter-event name; the existing line preserves the human-readable message for current log-readers. Edit at Pin-0.4 line 1767:

```rust
                        // mika#852 — counter-friendly structured event (stable
                        // name, suffix `_total` follows the tracing-counter
                        // convention). Emitted alongside the human-readable
                        // message below; both lines carry the same field shape
                        // so log-readers tailing either one continue to work.
                        // Future "do we need to debounce ready-label stalls?"
                        // can answer via:
                        //   jq 'select(.message == "ready_label_dispatch_stall_total")
                        //       | .timestamp' < $MIKA_SERVER_LOG_FILE
                        error!(
                            trace_id = %tool_ctx.trace_id,
                            location = %location,
                            label = mode.label(),
                            "ready_label_dispatch_stall_total"
                        );
                        error!(
                            trace_id = %tool_ctx.trace_id,
                            location = %location,
                            label = mode.label(),
                            "ready_label_dispatch_stalled — operator notification fired"
                        );
```

**Why dual-emission and not rename.** The existing line `"ready_label_dispatch_stalled — operator notification fired"` is the message any current log-tailer or alert-rule grep is keyed on. Renaming would silently break those consumers. Dual-emission costs one extra `error!` line per stall (the stall path itself is rare — exceptional, not hot path), and preserves backward compatibility for human-readable greps while adding the counter-friendly event name. Architect feedback at first pass acknowledged this as the right trade.

### Acceptance gate for AC3

- `grep -n 'ready_label_dispatch_stall_total' crates/mika-agent/src/agent.rs` returns at least one match in the stall path.
- The existing `"ready_label_dispatch_stalled — operator notification fired"` line is preserved (regression: log-tailer compatibility).
- Manual trace: if a stall path unit/eval test exists at agent.rs:7707+ (the test the original plan referenced), it captures both event names. If no such test exists today, AC3 ships without one — the stall-path emission is structurally simple (two `error!` calls in sequence) and the existing 100% test covers the dispatch-correctness side.

## 4. AC4 — Disambiguation sentence on Ready-Label Dispatch section

**Section location post-#1106 (Pin-0.9):** `skills/bundled/self-dev-webhook-ready-label/system_prompt.md`. The current opening (lines 1–5) is reproduced at Pin-0.9.

**Plan:** Insert a one-paragraph scope clarification between the existing second paragraph and the engine-enforces blockquote. New paragraph (~280 bytes):

```
**Scope:** Applies **only** when the label is exactly `ready`. For any other label (`bug`, `p1-important`, `needs-triage`, etc.), the engine routes the webhook to Webhook Fallthrough in the `self-dev` prompt — those labels are not dispatch consent and must not trigger `run_claude_pilot` or `run_claude_pilot_groom`.
```

Why this still has value despite the file separation from #1106: the dedicated skill activates on a *keyword* match (per `self-dev-webhook-ready-label/skill.toml` keyword config). If a future format-string change ever broadens the trigger surface (e.g., a new label-shape webhook arrives via a different marker), the prompt-level "applies only when label is exactly `ready`" sentence is the second-line defense against false-positive dispatch. The engine-level `is_ready_label_dispatch_marker` predicate (Pin-0.1) is the first line; this sentence is defense-in-depth on the LLM's interpretation layer.

### Acceptance gate for AC4

- `skills/bundled/self-dev-webhook-ready-label/system_prompt.md` contains the literal phrase `Applies **only** when the label is exactly` near the section opening.
- `bundled_skills_load_without_oversized_prompts` passes (the additional ~280 bytes is well below cap; current file size is small, well under the per-skill `max_prompt_size`).
- `bundled_skills_approaching_max_prompt_size_warns` (the new AC2 test) passes (still well below 95% of cap).

## 5. AC5 — Comment-harden the `ready_label_dispatch_satisfied` predicate

**Site:** `crates/mika-agent/src/agent.rs:5197–5219` (Pin-0.5). The existing doc comment at lines 5198–5211 explains *why* attempts (not successes) are counted but names only "(global_dispatch_active, task_not_dispatchable, etc.)" — the abbreviated form the ticket flags as future-staleness-prone.

**The current rejection set per `crates/mika-agent/CLAUDE.md § Dispatch-readiness guard (#525)` is seven:**

1. `unauthorized_webhook_dispatch` (mika#933, check 0)
2. `task_not_dispatchable` (check 1)
3. `task_active_dispatch` (check 2)
4. `global_dispatch_active` (check 3)
5. `dispatch_limit_exceeded` (check 4)
6. `dispatch_no_grooming_marker` (mika#919, check 5)
7. `dispatch_blocked_by` (mika#713, check 6)

**Plan:** Rewrite the doc comment to enumerate all seven. The new comment replaces lines 5198–5211 (preserves the History paragraph at 5206–5211 unchanged):

```rust
/// Returns `true` when `run_claude_pilot` or `run_claude_pilot_groom` was
/// attempted on this turn (success or failure). The
/// `webhook_ready_label_dispatch` intent-guard satisfies on **attempts**, not
/// **successes**, because the seven terminal rejections from
/// `validate_dispatch_readiness` (`crates/mika-agent/src/skills/executor.rs`)
/// are structural and not recoverable by re-prompting the LLM:
///
///   1. `unauthorized_webhook_dispatch` — non-ready webhook hit the prevention surface (mika#933)
///   2. `task_not_dispatchable` — task in a terminal state (`blocked`/`completed`/`cancelled`)
///   3. `task_active_dispatch` — same task already has an active callback child
///   4. `global_dispatch_active` — another task of the same dispatch class has an active callback (mika#583, #1001)
///   5. `dispatch_limit_exceeded` — per-turn dispatch counter already at the limit (mika#583)
///   6. `dispatch_no_grooming_marker` — ungroomed issue rejected at the gate (mika#919)
///   7. `dispatch_blocked_by` — open GitHub blockers remain (mika#713)
///
/// Re-prompting the LLM after any of these would loop (the LLM cannot dissolve
/// a structural rejection); the operator-notification path
/// (`agent.rs:1762-1784`) fires instead.
///
/// History: #907 added an OR-shape (`run_claude_pilot || send_message`) to
/// accept grooming-rejection notifications.  #996 replaced the rejection path
/// with auto-groom via `run_claude_pilot(dev-groom)`, making `send_message`
/// obsolete for this guard.  #1089 removed `send_message` from the predicate
/// after fabricated `check_task` pre-flights exploited the over-broad match to
/// short-circuit dispatch via a hallucinated escalation that hit NoChannel.
/// #1173 — dev-groom owns its own tool (`run_claude_pilot_groom`); both names
/// satisfy the dispatch contract.
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
```

**The redundant inline comment at lines 5213–5215** (`// mika#1173: dev-groom owns its own tool ...`) is folded into the History paragraph (last line) since the doc comment now names both tools by their canonical role. Body stays intact.

**Why all seven and not the four originally named in the ticket body.** The ticket body refresh of 2026-05-20 explicitly notes: *"all seven variants per CLAUDE.md, superset of the original four named at filing time."* A comment listing only the four would re-introduce the same staleness class the comment is meant to prevent. The duplication-vs-by-reference trade is resolved by enumeration — `validate_dispatch_readiness` is in a sibling crate file (`skills/executor.rs`), and the canonical list lives in `crates/mika-agent/CLAUDE.md § Dispatch-readiness guard (#525)`. A future PR that adds an 8th rejection variant updates **both** the executor enum, CLAUDE.md, and this comment — that's the same multi-site update the project already does for every dispatch-class change, and the discoverability gain (a future cleanup PR looking at the predicate reads the full list inline) outweighs the duplication cost.

### Acceptance gate for AC5

- The doc comment above `ready_label_dispatch_satisfied` at `crates/mika-agent/src/agent.rs` names all seven `validate_dispatch_readiness` rejection variants by their canonical error-string identifier.
- The four ticket-named cases (`global_dispatch_active`, `task_not_dispatchable`, `dispatch_blocked_by`, `dispatch_limit_exceeded`) are all present in the list — superset satisfies the ticket text.
- The History paragraph (#907, #996, #1089, #1173 lineage) is preserved.

## 6. Sequencing

The five items are independent. Suggested commit order (ascending blast radius):

1. **AC5 (comment)** — pure documentation, zero behavior change, easiest to verify in isolation.
2. **AC3 (log-event split)** — single-file diff in `agent.rs`, no test signal needed beyond compile-clean.
3. **AC4 (prompt sentence)** — single-file diff in `self-dev-webhook-ready-label/system_prompt.md`; `bundled_skills_load_without_oversized_prompts` transitively verifies.
4. **AC2 (95% warn gate)** — new test in `bundled_skills_load.rs` + `pub(crate)` exposure of `MAX_PROMPT_SNIPPET_SIZE`/`MAX_PROMPT_SIZE_CEILING`; touches no production code.
5. **AC1 (shared marker + cross-crate test)** — touches three crates (`mika-common`, `mika-agent`, `mika-gateway`); largest blast radius, last.

Each item lands as a separate commit on the branch. PR is one bundled PR (per the ticket structure and `feedback_implementation_scope_bundling`).

## 7. Out-of-band notes

- **Item from "Out of scope (file separately)":** the ticket explicitly defers the "process improvement" item (PR template "compound docs consulted" field, mika-arch critique-checklist update). NOT addressed in this PR; if it has not been filed as a separate ticket since 2026-05-09, that filing is itself out-of-scope for this PR but worth surfacing to the operator at PR-open time.
- **Skill-prompt drift watch:** AC2's warn gate at 95% will fire if any bundled skill's prompt grows ≥ 95% of its declared cap. Today's tightest skill is `self-dev/system_prompt.md` at 76.5% of 65,536 — 12 KB of growth headroom. Compound docs on "skill prompt growth pressure" do not exist; if AC2 starts firing in the future, the operator can either raise the cap toward the 81,920-byte ceiling or shard via `[dependencies]`.

## 8. Test plan

- `cargo build -p mika-common -p mika-agent -p mika-gateway` — compiles clean after AC1's symbol move.
- `cargo test -p mika-gateway test_format_event_text_issue_labeled_extracts_label_name` — augmented test passes (both new prefix-contract assertion and existing exact-shape assertion).
- `cargo test -p mika-agent --test bundled_skills_load` — both existing `bundled_skills_load_without_oversized_prompts` and new `bundled_skills_approaching_max_prompt_size_warns` pass.
- `cargo test -p mika-agent --lib webhook_dispatch::tests::` — existing Row A/B/etc tests unchanged (assertion still passes on the marker now sourced from `mika-common`).
- `cargo clippy --all-targets` — clean.

## 9. Acceptance checklist (verbatim from issue body)

- [ ] `READY_LABEL_DISPATCH_MARKER` is shared across crates **OR** a cross-crate fixture test asserts the prefix contract. Renaming the gateway formatter's output produces a compile error or test failure. → **§1, both legs taken (shared const in `mika-common` + augmented fixture test in `mika-gateway`).**
- [ ] CI test fails when any bundled skill's `system_prompt.md` exceeds 100% of `max_prompt_size`, warns at 95%. → **§2, new warn test added; 100% gate already exists in `bundled_skills_load_without_oversized_prompts`.**
- [ ] `ready_label_dispatch_stall_total` counter increments alongside the operator notification. → **§3, dedicated structured log event added (sibling line, dual-emission preserves backward compatibility).**
- [ ] Ready-Label Dispatch section opens with the "applies *only* when label is exactly `ready`" sentence. → **§4, sentence added at new section location (self-dev-webhook-ready-label per mika#1106).**
- [ ] `ready_label_dispatch_satisfied` has a code comment naming the current terminal-rejection cases (all seven variants per CLAUDE.md, superset of the original four). → **§5, all seven named with mika# citations and link to the operator-notification fallback path.**
