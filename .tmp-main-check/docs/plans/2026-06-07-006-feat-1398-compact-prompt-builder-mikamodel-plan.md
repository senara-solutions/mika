# Plan — feat(llm): compact prompt builder for ProviderKind::MikaModel (mika#1398)

## Phase 0 — Pin

Verbatim slices anchoring the fix sites:

**A. Prompt builder** (`crates/mika-agent/src/prompt.rs:479`):
```rust
pub fn build_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    write_soul_section(&mut prompt, ctx.soul_content);
    write_identity_section(&mut prompt, ctx.identity);
    write_time_section(&mut prompt, ctx.current_utc, ctx.timezone.as_deref());
    write_channel_section(&mut prompt, ctx.channel_type, ctx.telegram_configured);
    write_core_memory_section(&mut prompt, ctx.core_memory, ...);
```

**B. Non-test caller — main agent loop** (`crates/mika-agent/src/agent.rs:2661`):
```rust
let mut system = prompt::build_system_prompt(&prompt_ctx);
```

**C. Non-test caller — team-context branch** (`crates/mika-agent/src/agent.rs:4396`):
```rust
let mut system = prompt::build_system_prompt(&prompt_ctx);

// Inject team context after the base system prompt
system.push_str("\n## Team Context\n");
```

**D. Provider enum** (`crates/mika-common/src/llm/mod.rs:236`):
```rust
pub enum ProviderKind {
    Anthropic, OpenAi, OpenRouter, Groq, Ollama, Mistral, Google,
    DeepSeek, MiniMax, Kimi, Qwen,
    #[serde(rename = "mikamodel")]
    MikaModel,
}
```

## Problem

When the active provider is `ProviderKind::MikaModel`, the prompt-assembly path emits the full ~167KB system prompt with 63 sections (QA Review Skill ~49KB, Milestone Workflow ~21KB, Self-Dev Skill ~18KB, full tool catalog with 58 entries). The provider expects a minimal system prompt (~250 chars, single paragraph, ~5-10 tools max) and OOD-prior-dominates when given the full mika-spirit context — it completes the markdown structure instead of acting as an agent (emits fictional `## Summary / Completed Tasks / Pending` status reports unrelated to the user query).

## Why

PR#1380 added `ProviderKind::MikaModel` variant routing. The provider's runtime context-window expectation diverges 670× from the assembled prompt's typical size. Without a compact-prompt branch, the provider's outputs are structurally unrelated to user input — agent-mode is impossible.

## Approach

Single branching point at the call site. Two distinct prompt builders:

- `build_system_prompt(ctx)` — existing, unchanged. Used for Anthropic / OpenAI / Kimi / OpenRouter / Google / DeepSeek / MiniMax / Qwen / Groq / Ollama / Mistral.
- `build_compact_system_prompt(ctx)` — new. Emits ≤5KB system prompt with ≤3 sections:
  - `## Personality` (~100 chars) — soul summary or identity persona line
  - `## Identity` (~50 chars) — agent name + role
  - `## Tool Usage` (~optional) — only when tools present, lists what's directly callable on this turn (NOT the full 58-entry catalog)

Callers (agent.rs:2661 + agent.rs:4396) branch on `provider_kind`:

```rust
let mut system = if provider_kind == ProviderKind::MikaModel {
    prompt::build_compact_system_prompt(&prompt_ctx)
} else {
    prompt::build_system_prompt(&prompt_ctx)
};
```

The `provider_kind` is already available at both call sites via the agent's runtime config (resolved from `config.toml` / overrides).

## What's omitted in compact path

From `build_system_prompt`:
- Soul section (heavy markdown, optional in compact)
- Identity section (collapsed to a single line in compact)
- Time section (omitted — provider doesn't use)
- Channel section (omitted)
- Core memory section (omitted — too large; provider can't condition on it)
- Skills sections (omitted — eagerly-loaded skill bodies are the bulk of the 167KB)
- Tool catalog beyond directly-callable on this turn (omitted)
- KG context section (omitted)
- Instructions section (omitted)
- Communication channel section (omitted)

## Acceptance Criteria

1. **AC1: `ProviderKind::MikaModel` branches in prompt-assembly to emit ≤5KB system prompt with ≤3 sections (Personality + Identity + optional Tool Usage).**
   - Verify by reading the new `build_compact_system_prompt(ctx)` output for a typical `PromptContext` and asserting `prompt.len() <= 5120` and `prompt.matches("##").count() <= 3`.

2. **AC2: A/B verification — same `mika ask "hello"` probe before/after produces agent-mode response (greeting / clarifying question / tool call), not a fictional status-report document.**
   - Run pre-fix: `MIKA_LLM_PROVIDER=mikamodel MIKA_OLLAMA_DUMP_PAYLOAD=/tmp/mika-payload-pre.json mika --agent mikamodel-probe ask "hello"` — capture response.
   - Run post-fix: same command with `-post.json` — capture response.
   - Diff `/tmp/mika-payload-pre.json` and `-post.json` system fields: pre is ~167KB; post is ≤5KB.
   - Diff responses: pre is fictional status report; post is agent-mode (greeting / question / tool call).

3. **AC3: No regression on other providers.**
   - Existing prompt-assembly tests (`prompt.rs:1056-1356` test cases) continue to pass — they invoke `build_system_prompt(ctx)` directly, no provider parameter, no branch.
   - Manual probe with each non-MikaModel provider (Anthropic / OpenAI / Kimi) confirms response shape unchanged.

4. **AC4: Payload-dump (PR#1389) confirms the new compact shape in dev-mode probe.**
   - `MIKA_OLLAMA_DUMP_PAYLOAD` dump for `mikamodel-probe` agent shows: `system` field ≤5KB, `tools` array ≤10 entries (only directly-callable), `messages[0].content` preserved.

5. **AC5: Compact builder has at least one unit test asserting size + section count.**
   - Test at `prompt.rs` near `build_system_prompt` tests: `test_build_compact_system_prompt_size_bound`.

## Files to change

- `crates/mika-agent/src/prompt.rs` — add `build_compact_system_prompt(ctx: &PromptContext<'_>) -> String` near line 479; add 1+ unit tests.
- `crates/mika-agent/src/agent.rs:2661` — branch on `provider_kind`.
- `crates/mika-agent/src/agent.rs:4396` — same branch (team-context path).

## Cross-references for context

- Pin slice A — prompt.rs:479-490
- Pin slice B — agent.rs:2661
- Pin slice C — agent.rs:4396 + 4399-4401 (team context injection)
- Pin slice D — mika-common/src/llm/mod.rs:236-254 (ProviderKind enum)

## Out of scope (per ticket)

- Changes to other providers' prompt assembly
- Skill / Core-Memory / Tool catalog redesign
- Provider-specific context-window negotiation
- Removing `mikamodel-probe` agent (kept for post-fix A/B verification per ticket scope)

## Risk

Low. Single conditional at two call sites; existing `build_system_prompt` is unchanged. The new `build_compact_system_prompt` is additive. Existing tests don't need updates. The only structural risk is forgetting to branch at one of the two call sites — both must be updated together.

## Test plan

1. `cargo test -p mika-agent --lib prompt` — existing tests pass + new compact-prompt test passes.
2. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` — clean.
3. Manual A/B per AC2.
4. Manual non-regression check per AC3 with Anthropic provider probe.

## Implementation order

1. Add `build_compact_system_prompt(ctx)` function to `prompt.rs` (additive — no existing test changes).
2. Add unit test asserting size + section count.
3. Branch at `agent.rs:2661` first call site.
4. Branch at `agent.rs:4396` second call site.
5. Run AC2 A/B probe with `mikamodel-probe` agent.
6. Run AC3 non-regression probe with Anthropic.
