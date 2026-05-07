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

Add `max_tokens: Option<usize>` to the existing `[context.summary]` section in `identity.toml` (which Axis 4 introduced). The field is `Option` so unset means "no cap" (preserves current behavior). The field is **mode-agnostic on its name** — the silent-mode gating lives in code (`if silent_trigger.is_some()`), not in the field's identifier. This per architect first-pass F1 (Orthogonality/KISS): the field is "max_tokens applied when the in-code gate fires"; mixing the gate condition into the name would over-couple config schema to current call-site policy.

When set:

- `None` → behavior unchanged from current (or from Axis 4's `inject = false` if also set; Axis 4 wins by short-circuit).
- `Some(0)` → on silent-mode turns, summary is omitted entirely. **`Some(0)` is a load-omit sentinel, not a "zero-token cap" interpreted literally** — the code path for `Some(0)` is the same as Axis 4's `inject = false` short-circuit but conditional on silent mode. Calling out the sentinel meaning explicitly per architect first-pass NF1.
- `Some(n)` for n > 0 → on silent-mode turns, summary is loaded and truncated to approximately n tokens before injection. Non-silent turns load the full summary unchanged.

### Why this shape

**Why `Option` not `usize` with sentinel default.** A sentinel like `0 = unlimited` would conflict with `0 = omit`, and a u32::MAX sentinel is uglier than absent. `Option<usize>` reads as "optional cap" which is the actual semantic.

**Why `max_tokens` not `silent_mode_max_tokens`** (architect F1). The mode-gating belongs in code, not in the field identifier. Today the only call site applying the cap is the silent-mode injection path; tomorrow another call site (e.g., compaction-triggered re-summarization) might want the same cap with different gate logic. Naming the field for its current gate freezes that coupling. `max_tokens` says what it is (a token cap on summary content); the gate `if silent_trigger.is_some()` says when. Two orthogonal concerns, two separate names — per `feedback_orthogonality_flag_semantics`.

**Why nested under `[context.summary]` (already exists from Axis 4).** Axis 4's design explicitly anticipated a cap field as a sibling: see Axis 4 plan §"Why nested `[context.summary]`" — `budget_tokens` was the named example. Filing it under the same section is the no-rename, no-migration path Axis 4 specifically designed for.

**Why detect via `SilentTrigger` (not callback-mode or webhook-mode).** The existing `SilentTrigger` enum at `crates/mika-agent/src/agent.rs` (and its variant `SilentTrigger::DeferredDispatch` introduced in mika#1011) is the canonical structural marker for "this turn has no human in the loop and no streaming UI." Both callback and webhook events flow through `SilentTrigger`. The detection lives in code; the field stays mode-agnostic.

**Why token-cap semantics, not character-cap.** The leak is measured in tokens (the LLM's frame of reference). Approximating tokens via character count (4 chars ≈ 1 token in English) is acceptable for cap-enforcement; precise tokenization is not required because (a) the cap is heuristic, not a hard limit, and (b) tokenizers vary by provider and bringing one in just for cap-enforcement is overkill. The 4-char ratio is captured as a named constant `CHARS_PER_TOKEN_ESTIMATE = 4` (architect NF2) rather than a magic number, so a future maintainer encountering `max_chars = max_tokens * 4` doesn't have to re-derive the rationale.

### Rejected alternatives

- **Field on `Identity` top level (e.g., `summary_silent_mode_budget`).** Loses the `[context.summary]` grouping that Axis 4 introduced and which is the right home for any summary-control field.
- **Hardcoded budget in `agent.rs` with no config.** Removes operator control. Different agents (mika-arch is callback-dominant; mika-prime is interactive-dominant) want different policies.
- **Detect silent mode via messages-array-length heuristic** (`if messages.len() <= 2`). Heuristic shape; couples to message-shape that may change. The `SilentTrigger` enum is the structural marker designed for exactly this case.
- **Apply in summarizer (Axis 2 territory).** Axis 2 changes summary content/format. Axis 3 changes summary delivery conditional on call site. Different layers; both legitimate.

## Implementation Steps

### Step 1: Extend `ContextSummaryConfig` with `max_tokens` field + `CHARS_PER_TOKEN_ESTIMATE` constant

**File:** `crates/mika-agent/src/prompt.rs`

Add the named constant near the top of the module:

```rust
/// Heuristic conversion ratio for character-count to token-count approximation.
///
/// Used by `truncate_to_token_budget()` (and any other code path that needs to
/// cap summary content by approximate token count without invoking a real
/// tokenizer). The 4:1 ratio is acceptable for English and is conservative
/// enough for cap-enforcement; exact tokenization is not required because the
/// cap is a soft policy, not a hard limit, and tokenizers vary by provider.
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
```

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

    /// Optional token cap applied to the summary by the gated injection path.
    ///
    /// The cap is mode-agnostic at the schema level. Today the only caller
    /// applying it gates on `SilentTrigger::is_some()` — see
    /// `load_gated_summary()`. Tomorrow another gate could reuse the same field
    /// without a rename.
    ///
    /// When set:
    ///   - `Some(0)` → load-omit sentinel: summary is not injected. Same code
    ///     path as Axis 4's `inject = false` short-circuit, but conditional on
    ///     the in-code gate firing (e.g., silent-mode only). NOT interpreted as
    ///     "zero-token cap"; treated as a structural omit signal.
    ///   - `Some(n)` for n > 0 → summary truncated to approximately
    ///     `n * CHARS_PER_TOKEN_ESTIMATE` characters before injection.
    ///   - `None` → no cap (default; current behavior).
    ///
    /// Token approximation is heuristic via `CHARS_PER_TOKEN_ESTIMATE` (= 4).
    /// (Axis 3 — mika#1021)
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

impl Default for ContextSummaryConfig {
    fn default() -> Self {
        Self {
            inject: default_inject_summary(),
            max_tokens: None,
        }
    }
}
```

**Rationale:** Additive change. Existing `identity.toml` files without the new field deserialize correctly with `max_tokens = None`. No migration needed. Field name is mode-agnostic per architect F1; gate condition lives in `load_gated_summary()` (Step 2).

### Step 2: Extract `load_gated_summary()` shared helper + plumb into both sites

**Files:** `crates/mika-agent/src/prompt.rs` (helper definition) and `crates/mika-agent/src/agent.rs` (two call-site updates).

Per architect first-pass F2 (DRY): extract the Axis-4-then-Axis-3 gate sequence into one helper called from both injection sites. Without this, both call sites would duplicate (a) the `inject = false` short-circuit, (b) the `db.load_conversation_summary()` call, (c) the `silent_trigger`-conditional cap application, and (d) the `<context>` tag wrapping. With the helper, both sites call one function and get one consistent semantics.

Helper signature (lives in `prompt.rs` next to `ContextSummaryConfig`):

```rust
/// Load the conversational summary for injection into the system prompt,
/// applying Axis 4 (load-prevention) and Axis 3 (mode-conditional cap)
/// gates in sequence.
///
/// Returns `Ok(None)` when the summary should not be injected. Three reasons:
///   - `inject = false` (Axis 4 short-circuit; no DB call made)
///   - no summary stored in the DB
///   - silent mode + `max_tokens = Some(0)` (Axis 3 load-omit sentinel)
///
/// Returns `Ok(Some(content))` with the (possibly truncated) summary content
/// to inject. Caller is responsible for the surrounding `<context>` tag wrap
/// + section header.
///
/// The "gated" name signals: this is the policy-aware load path. Any direct
/// `db.load_conversation_summary()` callers (e.g., debugging, migration)
/// bypass the gates intentionally.
async fn load_gated_summary(
    db: &Database,
    summary_config: &ContextSummaryConfig,
    silent_trigger: Option<&SilentTrigger>,
) -> Result<Option<String>> {
    // Axis 4: hard load-prevention.
    if !summary_config.inject {
        return Ok(None);
    }

    // Load the summary; absence is not an error.
    let Some(summary) = db.load_conversation_summary().await? else {
        return Ok(None);
    };

    // Axis 3: mode-conditional cap. The mode gate is `silent_trigger.is_some()`;
    // the field name (`max_tokens`) is mode-agnostic.
    match (silent_trigger, summary_config.max_tokens) {
        (Some(_), Some(0)) => Ok(None),  // Silent + load-omit sentinel.
        (Some(_), Some(n)) => Ok(Some(truncate_to_token_budget(&summary.content, n))),
        _ => Ok(Some(summary.content)),  // Non-silent or no cap → full summary.
    }
}
```

Both summary-injection sites collapse to:

```rust
if let Some(content) = load_gated_summary(&db, &identity.context.summary, silent_trigger.as_ref()).await? {
    system.push_str("\n## Conversation Summary\n");
    system.push_str("<context type=\"summary\" trust=\"data\">\n");
    system.push_str(&content);
    system.push_str("\n</context>\n");
}
```

The `silent_trigger.as_ref()` argument is `None` at the conversation-mode call site (no in-scope `silent_trigger`), and `Some(_)` at the team/silent-mode call site. The helper handles both uniformly; the call sites stay parallel.

**Why one helper, not two.** A two-helper split (e.g., `load_summary_for_conversation()` + `load_summary_for_silent()`) would still duplicate the Axis 4 short-circuit and the load call. One helper that takes the silent-trigger as an `Option` parameter is the minimal surface that closes both DRY axes (the load call AND the gate sequence). Per architect Q3: this also bounds the `SilentTrigger` plumbing scope to one parameter, no cascade.

**Why the helper returns `Ok(None)` instead of an empty string for the omit case.** `Ok(None)` lets the caller skip the surrounding `<context>` tags and section header entirely — emitting `## Conversation Summary\n<context>...empty...</context>` would inject the framing without the content, which is its own form of leakage signal to the model.

**Orthogonality with Axis 4.** Axis 4's `inject = false` is strictly stronger (returns `Ok(None)` before any DB call). Axis 3's cap fires only when `inject = true` AND `max_tokens` is set AND silent mode is detected. The two gates are orthogonal: `inject` controls "load at all"; `max_tokens` controls "how much, when in mode." Satisfies AC#4 and `feedback_orthogonality_flag_semantics`.

### Step 3: Token-budget truncation helper

**File:** `crates/mika-agent/src/prompt.rs` (alongside `CHARS_PER_TOKEN_ESTIMATE` constant from Step 1).

```rust
/// Truncate a summary string to approximately `max_tokens`, using
/// `CHARS_PER_TOKEN_ESTIMATE` for the conversion. Cuts at a word boundary
/// near the budget to avoid mid-word truncation in the LLM's view. Appends
/// a truncation marker so the model knows content was elided.
///
/// Heuristic: not exact tokenization. Acceptable because the cap is a soft
/// policy, not a hard limit, and tokenizers vary by provider. The constant
/// `CHARS_PER_TOKEN_ESTIMATE` documents the conversion ratio in one place.
pub fn truncate_to_token_budget(summary: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(CHARS_PER_TOKEN_ESTIMATE);
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

**Rationale (per architect NF2):** the `CHARS_PER_TOKEN_ESTIMATE` constant defined in Step 1 replaces the magic `4` literal here. Future maintainers see the named constant and find its definition + rationale at the module top, not buried inside this helper.

Heuristic truncation at word boundary. Truncation marker is honest signal to the model and to a debugging operator reading the prompt log. Marker text is fixed (architect Q2: YAGNI — no observed need for variation).

### Step 4: Identity TOML update for at-risk agents (operator-deferred)

This step files no code change. After this PR ships, the operator can opt-in per agent:

```toml
# ~/.mika/agents/<agent>/identity.toml
[context.summary]
inject = true            # Axis 4 — keep summary on interactive turns
max_tokens = 1000        # Axis 3 — cap to ~1000 tokens when the silent-mode gate fires
```

Or a stricter policy:

```toml
[context.summary]
inject = true
max_tokens = 0           # Axis 3 — omit summary entirely when the silent-mode gate fires
```

The plan does NOT seed `well_known_agents.rs` with new defaults for any agent. Operator decides per-agent based on observed behavior. Document the field in `crates/mika-agent/CLAUDE.md` per AC#5; that's the only documentation touchpoint this PR makes.

## Test Strategy

### Unit tests for `load_gated_summary()`

1. **`max_tokens = None`, silent turn, summary present** → returns `Ok(Some(full))` (regression).
2. **`max_tokens = Some(0)`, silent turn** → returns `Ok(None)` (load-omit sentinel; Axis 3 fires).
3. **`max_tokens = Some(0)`, non-silent turn** → returns `Ok(Some(full))` (cap is gated by mode).
4. **`max_tokens = Some(n)`, silent turn, summary > n tokens** → returns `Ok(Some(truncated))` with truncation marker.
5. **`max_tokens = Some(n)`, silent turn, summary < n tokens** → returns `Ok(Some(full))`.
6. **Axis 4 wins (`inject = false` + any `max_tokens`)** → returns `Ok(None)` without DB call (verify via mock DB asserting `load_conversation_summary` was NOT called).
7. **No summary stored** (`inject = true`, `max_tokens = None`) → returns `Ok(None)`.

### Unit tests for `truncate_to_token_budget()` helper

8. **Boundary tests:** empty string, exactly `max_tokens * CHARS_PER_TOKEN_ESTIMATE` chars, `+1` over, very long with whitespace, very long without whitespace (no boundary → cuts at `max_chars`).

### Integration / eval tests

`crates/mika-agent/tests/eval/` already exercises the prompt-assembly seam. Add a scenario:
- Agent with `max_tokens = Some(500)` + a 5000-char summary fixture
- Trigger via `EvalHarness` callback path (silent)
- Assert `LlmRequest.system` contains the truncation marker + `system.len()` is bounded
- Negative-control variant: same agent, `max_tokens = None`, callback path → assert summary present in full

No real-LLM eval needed — the assertion is on prompt content, not LLM behavior.

## Acceptance Criteria

Mirroring the ticket body for traceability:

- **AC#1**: Silent-mode detection lives in `load_gated_summary()` via `Option<&SilentTrigger>` parameter. Both call sites (`agent.rs:2018-2024` conversation-mode and `agent.rs:3044-3049` team-mode) pass the in-scope `silent_trigger.as_ref()`. The mode-gating predicate is `silent_trigger.is_some()`, structurally tied to the canonical `SilentTrigger` enum.
- **AC#2**: Silent-mode + `max_tokens = Some(0)` → summary omitted (load-omit sentinel). Silent-mode + `max_tokens = Some(n)` for n > 0 → summary truncated to ~n tokens with marker. Verifiable via unit tests #2 and #4.
- **AC#3**: Non-silent mode + any `max_tokens` value → summary unchanged. Verifiable via unit tests #3 and #5 + existing eval scenarios pass.
- **AC#4**: Axis 4's `inject = false` short-circuits before any DB call OR cap evaluation. Verifiable via unit test #6 (mock DB asserting `load_conversation_summary` not called when `inject = false`). Orthogonality flag per `feedback_orthogonality_flag_semantics`.
- **AC#5**: `crates/mika-agent/CLAUDE.md` documents the `[context.summary].max_tokens` field, the load-omit sentinel semantics for `Some(0)`, the `CHARS_PER_TOKEN_ESTIMATE` heuristic, and the orthogonality with `[context.summary].inject`. Update lives under § Conventions or § Three-Layer Memory Model.

## Risks & Open Questions

- **R1 (low):** `SilentTrigger` plumbing into `load_gated_summary()` is bounded to one parameter (per architect Q3 answered). The two call sites in `agent.rs` already have `silent_trigger` in scope (the conversation-mode path has `None`; the team-mode path has `Some(_)`); no signature cascade beyond those two sites. Mitigation: scoped grep during implementation; if a third call site is found, decide between (a) routing it through the helper or (b) leaving it ungated with a comment.
- **R2 (low):** Heuristic 4-char-per-token approximation is conservative for English. For non-English languages the approximation skews. Acceptable because (a) the budget is a soft policy, (b) the only callers writing summaries today are the existing summarizer which produces English, and (c) the conversion ratio is captured as the named `CHARS_PER_TOKEN_ESTIMATE` constant so a future maintainer can swap the implementation in one place. If multi-language summaries become common, revisit with a tokenizer-based cap (separate ticket).
- **R3 (low):** A future Axis 2 (summarizer content reform) may make summary content less leak-prone, reducing the need for Axis 3. Axis 3 still has independent value (defense-in-depth + per-agent policy control). No architectural conflict.

**Resolved by architect first-pass:**
- Q1 (default policy): `None` is correct — additive, opt-in, no built-in default cap.
- Q2 (truncation marker text): fixed text is correct — YAGNI; no observed need for variation.
- Q3 (`SilentTrigger` plumbing scope): bounded to one parameter via `load_gated_summary()` — see R1.

## Sources

- mika#1009 finding doc: `mika/docs/solutions/best-practices/mika-arch-init-context-leakage-2026-05-06.md`
- mika#1019 (Axis 4 sibling) — closed via PR #1019 on 2026-05-07T17:39Z
- mika#1019 plan: `mika/docs/plans/2026-05-07-005-feat-prompt-inject-summary-opt-out-plan.md`
- `crates/mika-agent/src/agent.rs:2018-2024` (conversation-mode summary injection)
- `crates/mika-agent/src/agent.rs:3044-3049` (team/silent-mode summary injection)
- `crates/mika-agent/src/prompt.rs:353-387` (existing `ContextSummaryConfig` from Axis 4)
- `SilentTrigger` enum (existing structural silent-mode marker)
- 2026-05-07 orchestrator handoff carry-forward (mika#1009 axes 3/2/1 sequencing)
