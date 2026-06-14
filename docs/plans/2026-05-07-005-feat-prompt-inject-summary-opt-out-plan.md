---
ticket: mika#1016
type: feat
module: mika-agent
tags: [prompt-assembly, identity, context-channel-leakage]
parent: mika#1009
---

# Plan: Per-Agent `[context.summary].inject` Opt-Out Flag

## Problem

mika#1009 identified conversational summary injection as the load-bearing leak source for mika-arch's context-channel leakage. The summary — generated from prior conversation turns — leaks operational detail (dispatch decisions, error discussions) into the architect's system prompt, contaminating its reasoning. Axis 4 of the 4-axis fix plan is the smallest first ship: a per-agent config flag to **load-prevent** (not just skip-inject) the summary entirely.

**Load-prevention vs. injection-prevention (security framing).** This is a load-prevention gate: when the flag is `false`, `db.load_conversation_summary()` is not called, the summary is not deserialized, and the result is not held in the turn's local scope. This is strictly stronger than injection-prevention (load-the-summary-but-skip-the-push-str) and is the correct shape for mika#1009's leak class. Naming the property explicitly matters: a future maintainer who refactors `agent.rs` and sees `load_conversation_summary()` elsewhere in the same function must understand that re-introducing the call also re-introduces the leak. The name of the protection is "load-prevention," not "skip-inject."

## Design

Add a nested `[context.summary]` section to `identity.toml` with a single boolean field `inject`, defaulting to `true` (preserves existing behavior for all agents). When `false`, the two summary-injection sites in `agent.rs` short-circuit before the `db.load_conversation_summary()` call.

### Why nested `[context.summary]` (not flat `[context]`)

The flat `[context] inject_summary = false` shape would require renaming the field the moment Axis 3 ships (which adds a summary-budget cap as a sibling toggle). Nesting now:

```toml
[context.summary]
inject = false
# Future Axis 3:
# budget_tokens = 8000
```

…lets Axis 3 add a sibling field with no rename, no migration, and the section name carries the "summary" context so the field name `inject` reads naturally. Pre-empts the structural cost the ticket body explicitly anticipates ("Axis 3" deferred but named).

Rejected alternatives:
- **Field on `[kg]`**: KG and prompt-assembly are orthogonal concerns. Mixing them creates a slippery slope where any prompt-assembly toggle ends up inside `[kg]`.
- **Top-level field on `Identity`**: No structural home for related controls (Axis 3 budget would have to be a top-level `summary_budget_tokens`, which is uglier than `[context.summary] budget_tokens`).

## Implementation Steps

### Step 1: Add `ContextIdentityConfig` + `ContextSummaryConfig` and wire into `Identity`

**File:** `crates/mika-agent/src/prompt.rs`

1. Add the nested-section structs following the existing `KgIdentityConfig` / `ToolsIdentityConfig` pattern:

```rust
/// Per-agent context-injection config from `[context]` section of `identity.toml`.
///
/// Top-level holder for prompt-assembly toggles. Each context block (summary,
/// future: tools, system, memory) gets its own nested section.
///
/// Use case: well-known agents (mika-arch) where a specific context block is
/// a known leak source (mika#1009). Operator opts the agent out via
/// `identity.toml`; production behavior changes on next mika-spirit restart.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ContextIdentityConfig {
    #[serde(default)]
    pub summary: ContextSummaryConfig,
}

/// `[context.summary]` subsection — controls injection of the conversational
/// summary block into the system prompt at prompt-assembly time.
///
/// `inject = false` is a load-prevention gate, not injection-prevention:
/// `db.load_conversation_summary()` is not called, the summary is not
/// deserialized, and the result is not available to any downstream code path
/// in the same turn. This is the correct shape for mika#1009's leak class.
#[derive(Debug, Deserialize, Clone)]
pub struct ContextSummaryConfig {
    /// Whether to load and inject the conversational summary into the system prompt.
    /// Default: `true` (preserves current behavior for all existing agents).
    /// Set to `false` for agents where summary leakage is a known problem.
    #[serde(default = "default_inject_summary")]
    pub inject: bool,
}

fn default_inject_summary() -> bool {
    true
}

impl Default for ContextSummaryConfig {
    fn default() -> Self {
        Self {
            inject: default_inject_summary(),
        }
    }
}
```

2. Add `context: ContextIdentityConfig` field to the `Identity` struct (after `tools`):

```rust
#[serde(default)]
pub context: ContextIdentityConfig,
```

3. Add to `Identity::default()`:

```rust
context: ContextIdentityConfig::default(),
```

**Rationale:** Follows the exact same pattern as `KgIdentityConfig`, `SkillsIdentityConfig`, `ToolsIdentityConfig`. The `#[serde(default)]` attribute on `context` (top-level) and on `summary` (nested) means existing `identity.toml` files without `[context]` or without `[context.summary]` parse correctly with `inject = true` at every level.

### Step 2: Gate summary injection in conversation mode (load-prevention)

**File:** `crates/mika-agent/src/agent.rs:2019`

Change:

```rust
// Inject conversation summary into system prompt if one exists
if let Some(summary) = db.load_conversation_summary().await? {
    system.push_str("\n## Conversation Summary\n");
    system.push_str("<context type=\"summary\" trust=\"data\">\n");
    system.push_str(&summary.content);
    system.push_str("\n</context>\n");
}
```

To:

```rust
// Load-prevention gate: when [context.summary].inject is false, skip the
// db.load_conversation_summary() call entirely so the summary is never
// deserialized into this turn's scope (mika#1009 leak protection).
if ctx.identity.context.summary.inject {
    if let Some(summary) = db.load_conversation_summary().await? {
        system.push_str("\n## Conversation Summary\n");
        system.push_str("<context type=\"summary\" trust=\"data\">\n");
        system.push_str(&summary.content);
        system.push_str("\n</context>\n");
    }
}
```

**Why the outer-block wrap (load-prevention) and not just gating the `push_str` calls (injection-prevention):** With the outer wrap, the DB call does not happen and the `Summary` struct is never instantiated. Any future code path in the same function (logging, an audit dump, a conditional re-injection) cannot accidentally re-leak the summary because it is not in scope. The naming `load-prevention gate` is the load-bearing security property and must be preserved by future refactors.

### Step 3: Gate summary injection in silent mode (load-prevention)

**File:** `crates/mika-agent/src/agent.rs:3078`

Same transformation as Step 2 — wrap the existing `if let Some(summary) = db.load_conversation_summary()...` block in `if ctx.identity.context.summary.inject { ... }`. Same load-prevention semantics. Symmetric with conversation mode (per architect ratification NF2: gating both sites avoids creating an asymmetry that would confuse future readers and pre-empts the gap if mika-arch ever acquires a silent-mode trigger).

### Step 4: Update mika-arch's well-known agent identity

**File:** `crates/mika-agent/src/well_known_agents.rs:303` (function `build_mika_arch_identity`; `[kg]` block emit at line 338)

Update the formatted identity string to include the `[context.summary]` section:

```rust
Ok(format!(
    r#"name = "Architect"
emoji = "🏛"

[kg]
enabled = true
docs_roots = [
{roots_block}]

[context.summary]
inject = false

[skills]
allowlist = ["mika-arch-groom-ticket", "mika-arch-groom-milestone", "mika-arch-second-review"]

[tools]
{tools_block}
"#
))
```

**Why only mika-arch:** Per mika#1009, mika-arch is the confirmed leak source. mika-dev and mika-qa retain the default (`inject = true`) — their summary context is useful for maintaining dispatch continuity across turns.

### Step 5: Add tests

**File:** `crates/mika-agent/src/prompt.rs` (inline `#[cfg(test)] mod tests`)

1. **Config parsing — explicit false:** Verify `identity.toml` with `[context.summary] inject = false` parses correctly into `Identity { context: ContextIdentityConfig { summary: ContextSummaryConfig { inject: false } } }`.
2. **Config parsing — default (no `[context]` section):** Verify `identity.toml` without any `[context]` section defaults to `inject = true`.
3. **Config parsing — `[context]` present, no `[context.summary]`:** Verify that `[context]` with no nested `[context.summary]` still defaults `inject = true` (covers the partial-section case).
4. **Config parsing — explicit true:** Verify `[context.summary] inject = true` parses correctly.

**File:** `crates/mika-agent/src/well_known_agents.rs` (inline test module — alongside existing `test_build_mika_arch_identity*` tests around line 800)

5. **Provisioning test (NF1):** `test_build_mika_arch_identity_load_prevents_summary` — calls `build_mika_arch_identity(&test_settings_with_kg_roots())`, parses the result via `toml::from_str::<Identity>()`, asserts `identity.context.summary.inject == false`. Protects AC#4 against future format-string refactors that could silently drop the `[context.summary]` block. ~10 lines.

**File:** `crates/mika-agent/tests/eval/` (eval-harness scenario)

6. **Conversation-mode load-prevention scenario:** Use `EvalHarness` + `MockLlmProvider` with an identity overriding `[context.summary].inject = false`. Seed a conversation summary in the DB. Run a conversation turn. Assert: (a) the system prompt sent to the mock does NOT contain `## Conversation Summary`, AND (b) the test exercises `agent.rs:2019` (conversation-mode site).
7. **Conversation-mode default scenario:** Same setup but identity uses default (`inject = true`). Assert the system prompt DOES contain `## Conversation Summary`.

**Silent-mode coverage clarification (NF3):** The silent-mode injection at `agent.rs:3078` is on a path that requires triggering a silent-mode turn (e.g., heartbeat or callback resume). If the existing `EvalHarness` invocation surface does not expose a clean way to trigger a silent-mode turn, the silent-mode site is covered transitively by the field-access symmetry — both call sites read `ctx.identity.context.summary.inject` from the same struct, so the unit-level config parsing tests (#1-4 above) plus the conversation-mode eval scenario (#6-7) suffice. If `EvalHarness` does support silent-mode triggers (verify during implementation), add a third eval scenario that triggers a silent-mode turn and asserts the same prompt-content invariants. The implementer must explicitly note in the PR description which path was taken.

### Step 6: Update documentation

**File:** `crates/mika-agent/CLAUDE.md`

1. Add a new section "Context Injection Configuration" under § Conventions:

```markdown
## Context Injection Configuration

`[context]` section in `identity.toml` controls prompt-assembly behavior for context blocks. Each context block has its own nested subsection.

### `[context.summary].inject` (bool, default: `true`)

When `true`, the conversational summary (from compaction) is loaded from the
DB and injected into the system prompt as `<context type="summary" trust="data">`.
When `false`, the summary is **load-prevented** — `db.load_conversation_summary()`
is not called, the summary is not deserialized, and is not available to any
downstream code path in the turn. This is strictly stronger than
injection-prevention and is the correct shape for context-leakage protection.

Use case: agents where the conversational summary is a known context-channel leak source
(mika#1009). mika-arch is provisioned with `[context.summary] inject = false` by default.

```toml
[context.summary]
inject = false
```
```

2. **§ Three-Layer Memory Model annotation (NF4):** Add one sentence to the existing § Three-Layer Memory Model section (where context priority is listed: "current user message > core memory > active skill context > conversation summary > conversation history > search results"):

   > **Per-agent override:** mika-arch sets `[context.summary] inject = false`, removing the *conversation summary* layer from its system prompt entirely (mika#1009 leak protection). New agents that disable summary injection should be listed here.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/prompt.rs` | Add `ContextIdentityConfig` + `ContextSummaryConfig` structs; add `context` field to `Identity` |
| `crates/mika-agent/src/agent.rs` | Load-prevention gate at 2 sites (lines 2019, 3078) on `ctx.identity.context.summary.inject` |
| `crates/mika-agent/src/well_known_agents.rs` | Add `[context.summary] inject = false` to mika-arch identity; add provisioning test |
| `crates/mika-agent/src/prompt.rs` (tests) | Config parsing tests (4 scenarios: default, partial, explicit-true, explicit-false) |
| `crates/mika-agent/tests/eval/` | Conversation-mode load-prevention + default eval scenarios; silent-mode coverage note |
| `crates/mika-agent/CLAUDE.md` | New § Context Injection Configuration; § Three-Layer Memory Model annotation |
| Root `CLAUDE.md` (verify in Phase 0) | If a § Three-Layer Memory Model or equivalent context-priority section exists at the repo root, mirror the per-agent override annotation. If absent, document the no-op explicitly in the PR description. (Architect NF6: pre-commit discovery via `grep -rn "Three-Layer Memory Model" CLAUDE.md`.) |

## Issue-body sync (PR-time, NF5)

The mika#1016 issue body's AC text (AC#1–AC#4) uses the pre-pass-1 flat-section schema (`[context].inject_summary`) because the ticket was filed before grooming. The plan reframes that with sound architectural reasoning (F1: nested `[context.summary] inject`). Per the issue-as-versioned-contract discipline, the implementing PR must:

1. Update the issue body's AC text in-place to match the implemented schema (`[context.summary].inject`), preserving the original AC numbering.
2. Carry an edit-notice comment on the issue: *"AC text updated 2026-05-07 to match the post-grooming nested-section schema (mika-arch F1). Plan + commit history is the authoritative trail."*
3. Cross-reference this plan's commit SHA in the closure annotation when the PR lands.

This is a PR-author / operator action, not an architect or implementer ambiguity. Named here so it does not get lost.

## Risks

1. **Existing identity.toml files:** Low risk. `#[serde(default)]` on the `context` field (top-level) and on `summary` (nested) means all existing files parse without change. Default is `inject = true` = no behavior change.
2. **Silent mode agents losing context:** mika-arch doesn't use silent-mode heartbeat turns, so gating silent mode too is safe and symmetric. If a future agent opts out and relies on heartbeat, the operator would notice immediately (heartbeat turns with no summary context would be less useful but not broken).
3. **Test coverage:** The summary injection paths currently have no dedicated tests. This PR adds the first tests for this code path, which is a net positive. NF1 provisioning test specifically guards AC#4 against future format-string refactors that could silently drop the section.
4. **EvalHarness silent-mode trigger surface:** If `EvalHarness` does not expose silent-mode triggers, the silent-mode site (agent.rs:3078) is covered transitively via field-access symmetry — see Step 5 NF3 clarification. PR description must call out which path was taken.

## Out of Scope

- **Axis 3** (summary budget cap): Will land as `[context.summary] budget_tokens = N` in a future PR. The nested-section design preempts the rename cost.
- **Axis 2** (summarizer content reform): Separate concern, separate ticket.
- **Axis 1** (anti-conversational wrapper): Likely skippable per #1009.
