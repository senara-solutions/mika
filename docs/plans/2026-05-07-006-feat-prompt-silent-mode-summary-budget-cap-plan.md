---
ticket: mika#1021
type: feat
module: mika-agent
tags: [prompt-assembly, identity, context-channel-leakage, silent-mode]
parent: mika#1009
sibling: mika#1019
---

# Plan: Silent-Mode Summary Budget Cap (mika#1009 Axis 3)

## Problem

mika#1009's finding doc identified a 300:1 system-prompt-to-messages token ratio in silent-mode (callback / webhook event) turns: the messages array is a single trigger event (often a few hundred tokens) while the system prompt — including the conversational summary — can run thousands of tokens. At this ratio the LLM treats the summary as if it were prior conversation history and produces degenerate responses referencing "prior turns" the model never participated in.

Axis 4 (mika#1019, shipped 2026-05-07) added per-agent **load-prevention** of the summary block via `[context.summary].inject = false`. Axis 4 is the right shape for agents whose typical operation pattern is silent-mode-dominant (mika-arch). It is not the right shape for agents that benefit from summary continuity in interactive turns but get burned by it in silent turns — those agents need a **mode-conditional** gate, not a global opt-out.

This plan adds Axis 3: a silent-mode-only budget cap that short-circuits the same load-prevention path Axis 4 introduced, but conditional on the call site being a silent trigger. Agents keep the summary on streaming/CLI turns and lose it on callback/webhook turns where the ratio is dangerous.

## Design

Add `silent_mode_max_tokens: Option<usize>` to the existing `[context.summary]` section in `identity.toml` (which Axis 4 introduced). The field is `Option` so unset means "no cap" (preserves current behavior). When set:

- `None` → behavior unchanged from current (or from Axis 4's `inject = false` if also set; Axis 4 wins by short-circuit).
- `Some(0)` → silent-mode summary is omitted entirely (load-prevention; same code path as Axis 4 but conditional).
- `Some(n)` → silent-mode summary is loaded but truncated to ~n tokens before injection. Non-silent turns load the full summary unchanged.

### Why this shape

**Why `Option` not `usize` with sentinel default.** A sentinel like `0 = unlimited` would conflict with `0 = omit`, and a u32::MAX sentinel is uglier than absent. `Option<usize>` reads as "optional cap" which is the actual semantic.

**Why nested under `[context.summary]` (already exists from Axis 4).** Axis 4's design explicitly anticipated this field as a sibling: see Axis 4 plan §"Why nested `[context.summary]`" — `budget_tokens` was the named example. Filing it under the same section is the no-rename, no-migration path Axis 4 specifically designed for.

**Why "silent-mode" not "callback-mode" or "webhook-mode".** The existing `SilentTrigger` enum at `crates/mika-agent/src/agent.rs` (and its variant `SilentTrigger::DeferredDispatch` introduced in mika#1011) is the canonical structural marker for "this turn has no human in the loop and no streaming UI." Both callback and webhook events flow through `SilentTrigger`. Naming the field `silent_mode_max_tokens` aligns with the existing engine vocabulary. A future maintainer searching for "silent" finds both the enum and the config.

**Why token-cap semantics, not character-cap.** The leak is measured in tokens (the LLM's frame of reference). Approximating tokens via character count (4 chars ≈ 1 token in English) is acceptable for cap-enforcement; precise tokenization is not required because (a) the cap is heuristic, not a hard limit, and (b) tokenizers vary by provider and bringing one in just for cap-enforcement is overkill. Document the approximation in the field's doc comment.

### Rejected alternatives

- **Field on `Identity` top level (e.g., `summary_silent_mode_budget`).** Loses the `[context.summary]` grouping that Axis 4 introduced and which is the right home for any summary-control field.
- **Hardcoded budget in `agent.rs` with no config.** Removes operator control. Different agents (mika-arch is callback-dominant; mika-prime is interactive-dominant) want different policies.
- **Detect silent mode via messages-array-length heuristic** (`if messages.len() <= 2`). Heuristic shape; couples to message-shape that may change. The `SilentTrigger` enum is the structural marker designed for exactly this case.
- **Apply in summarizer (Axis 2 territory).** Axis 2 changes summary content/format. Axis 3 changes summary delivery conditional on call site. Different layers; both legitimate.

## Implementation Steps

### Step 1: Extend `ContextSummaryConfig` with `silent_mode_max_tokens`

**File:** `crates/mika-agent/src/prompt.rs`

Extend the struct introduced in mika#1019:

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct ContextSummaryConfig {
    /// Whether to load and inject the conversational summary into the system prompt.
    /// Default: `true` (preserves current behavior for all existing agents).
    /// Set to `false` for agents where summary leakage is a known problem on every turn.
    /// (Axis 4 — mika#1019)
    #[serde(default = "default_inject_summary")]
    pub inject: bool,

    /// Optional token budget applied to the summary on silent-mode turns
    /// (callback/webhook events with no streaming UI). When set:
    ///   - `Some(0)` → summary omitted entirely on silent turns.
    ///   - `Some(n)` → summary truncated to ~n tokens (≈ 4n characters) before injection.
    ///   - `None`    → no cap (default; current behavior).
    /// Non-silent turns ignore this field and inject the full summary.
    /// Token approximation is heuristic (4 chars ≈ 1 token); exact tokenization
    /// is not required for cap-enforcement.
    /// (Axis 3 — mika#1021)
    #[serde(default)]
    pub silent_mode_max_tokens: Option<usize>,
}

impl Default for ContextSummaryConfig {
    fn default() -> Self {
        Self {
            inject: default_inject_summary(),
            silent_mode_max_tokens: None,
        }
    }
}
```

**Rationale:** Additive change. Existing `identity.toml` files without the new field deserialize correctly with `silent_mode_max_tokens = None`. No migration needed.

### Step 2: Plumb silent-mode signal into prompt assembly

**File:** `crates/mika-agent/src/agent.rs`

Locate the two summary-injection sites (per mika#1009 finding):

1. **Conversation mode** at `agent.rs:2018-2024`
2. **Team/silent mode** at `agent.rs:3044-3049`

For both sites, plumb the in-scope `silent_trigger: Option<&SilentTrigger>` (or equivalent silent-mode signal) into the gating logic. The conversation-mode path is non-silent by definition — `silent_trigger` is `None` and the cap is bypassed. The team/silent path has `Some(SilentTrigger::*)` — apply the cap.

Gating logic (after Axis 4's `inject` check passes):

```rust
// Axis 3 — silent-mode summary budget cap (mika#1021)
let summary_to_inject = match (silent_trigger, identity.context.summary.silent_mode_max_tokens) {
    (Some(_), Some(0)) => None,                                   // Silent + budget=0 → omit
    (Some(_), Some(n)) => Some(truncate_to_token_budget(&summary, n)),  // Silent + cap → truncate
    _ => Some(summary),                                            // Non-silent or no cap → unchanged
};

if let Some(content) = summary_to_inject {
    system.push_str("\n## Conversation Summary\n");
    system.push_str("<context type=\"summary\" trust=\"data\">\n");
    system.push_str(&content);
    system.push_str("\n</context>\n");
}
```

**Why gate after the `inject` check, not before.** Axis 4's `inject = false` is strictly stronger (load-prevention skips the `db.load_conversation_summary()` call entirely; Axis 3's cap operates on already-loaded content). When `inject = false`, control never reaches Axis 3's gate — Axis 4 wins by short-circuit, satisfying the orthogonality requirement (AC#4).

### Step 3: Token-budget truncation helper

**File:** `crates/mika-agent/src/prompt.rs` (or a new `crates/mika-agent/src/prompt/budget.rs` if `prompt.rs` is already large)

```rust
/// Truncate a summary string to approximately `max_tokens`, using the
/// 4-chars-per-token heuristic. Cuts at a word boundary near the budget
/// to avoid mid-word truncation in the LLM's view. Appends a truncation
/// marker so the model knows content was elided.
///
/// Heuristic: not exact tokenization. Acceptable because the cap is a
/// soft policy, not a hard limit, and tokenizers vary by provider.
pub fn truncate_to_token_budget(summary: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(4);
    if summary.len() <= max_chars {
        return summary.to_string();
    }
    // Cut at the last word boundary at or before `max_chars`.
    let cut = summary[..max_chars]
        .rfind(char::is_whitespace)
        .unwrap_or(max_chars);
    let mut truncated = summary[..cut].to_string();
    truncated.push_str("\n[… summary truncated to fit silent-mode budget …]");
    truncated
}
```

**Rationale:** Heuristic truncation at word boundary. Truncation marker is honest signal to the model and to a debugging operator reading the prompt log.

### Step 4: Identity TOML update for at-risk agents (operator-deferred)

This step files no code change. After this PR ships, the operator can opt-in per agent:

```toml
# ~/.mika/agents/<agent>/identity.toml
[context.summary]
inject = true                       # Axis 4 — keep summary on interactive turns
silent_mode_max_tokens = 1000       # Axis 3 — cap to ~1000 tokens on silent turns
```

Or a stricter policy:

```toml
[context.summary]
inject = true
silent_mode_max_tokens = 0          # Axis 3 — omit summary entirely on silent turns
```

The plan does NOT seed `well_known_agents.rs` with new defaults for any agent. Operator decides per-agent based on observed behavior. Document the field in `crates/mika-agent/CLAUDE.md` per AC#5; that's the only documentation touchpoint this PR makes.

## Test Strategy

### Unit tests

1. **`silent_mode_max_tokens = None`, silent turn** → summary unchanged (regression).
2. **`silent_mode_max_tokens = Some(0)`, silent turn** → summary absent in prompt.
3. **`silent_mode_max_tokens = Some(0)`, non-silent turn** → summary present (cap is silent-only).
4. **`silent_mode_max_tokens = Some(n)`, silent turn, summary > n tokens** → truncation marker present in prompt.
5. **`silent_mode_max_tokens = Some(n)`, silent turn, summary < n tokens** → summary unchanged.
6. **Axis 4 wins:** `inject = false` + `silent_mode_max_tokens = Some(100)` → summary entirely absent (load-prevention shortcut).
7. **`truncate_to_token_budget` boundary tests:** empty string, exactly max_chars, max_chars + 1, very long with whitespace, very long without whitespace.

### Integration / eval tests

`crates/mika-agent/tests/eval/` already exercises the prompt-assembly seam. Add a scenario:
- Agent with `silent_mode_max_tokens = Some(500)` + a 5000-char summary fixture
- Trigger via `EvalHarness` callback path (silent)
- Assert `LlmRequest.system` contains truncation marker + `system.len()` is bounded

No real-LLM eval needed — the assertion is on prompt content, not LLM behavior.

## Acceptance Criteria

Mirroring the ticket body for traceability:

- **AC#1**: Silent-mode is structurally detected via the existing `SilentTrigger` signal in `build_system_prompt()`. The predicate flows through to the gating logic in both conversation-mode (`agent.rs:2018-2024`) and team-mode (`agent.rs:3044-3049`) paths.
- **AC#2**: Silent-mode + summary tokens > budget → summary is omitted (when `Some(0)`) or truncated to budget (when `Some(n)`). Verifiable via unit tests #2 and #4 above.
- **AC#3**: Non-silent mode → behavior unchanged from current. Regression tests #1 and #3 above + existing eval scenarios pass.
- **AC#4**: When Axis 4's `inject = false`, Axis 3's budget logic is short-circuited. Verifiable via test #6. Orthogonality flag per `feedback_orthogonality_flag_semantics`.
- **AC#5**: `crates/mika-agent/CLAUDE.md` documents the new field + the heuristic truncation contract under § Conventions or § Three-Layer Memory Model.

## Risks & Open Questions

- **R1 (low):** The `SilentTrigger` enum may not be in scope at exactly the line of the summary-injection sites. If plumbing requires a function-signature change, additional callers may need updating. Mitigation: scoped grep for callers during Step 2; widen the change if needed.
- **R2 (low):** Heuristic 4-char-per-token approximation is conservative for English (English tends toward ~4 chars/token in practice). For other languages the approximation skews. Acceptable because (a) the budget is a soft policy and (b) the only callers writing summaries today are the existing summarizer which produces English. If multi-language summaries become common, revisit with a tokenizer-based cap (separate ticket).
- **R3 (low):** A future Axis 2 (summarizer content reform) may make summary content less leak-prone, reducing the need for Axis 3. Axis 3 still has independent value (defense-in-depth + per-agent policy control). No architectural conflict.
- **OQ1 (groom):** Is the right default `silent_mode_max_tokens = None` (no cap unless opted-in, current shape) or a small built-in cap (e.g., `Some(2000)`) shipping as-default for all agents? The plan assumes `None` (additive, opt-in). Architect: confirm or override.
- **OQ2 (groom):** Should the truncation marker text be configurable, or is a fixed `[… summary truncated to fit silent-mode budget …]` good enough? Plan assumes fixed.

## Sources

- mika#1009 finding doc: `mika/docs/solutions/best-practices/mika-arch-init-context-leakage-2026-05-06.md`
- mika#1019 (Axis 4 sibling) — closed via PR #1019 on 2026-05-07T17:39Z
- mika#1019 plan: `mika/docs/plans/2026-05-07-005-feat-prompt-inject-summary-opt-out-plan.md`
- `crates/mika-agent/src/agent.rs:2018-2024` (conversation-mode summary injection)
- `crates/mika-agent/src/agent.rs:3044-3049` (team/silent-mode summary injection)
- `crates/mika-agent/src/prompt.rs:353-387` (existing `ContextSummaryConfig` from Axis 4)
- `SilentTrigger` enum (existing structural silent-mode marker)
- 2026-05-07 orchestrator handoff carry-forward (mika#1009 axes 3/2/1 sequencing)
