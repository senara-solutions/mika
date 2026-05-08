---
ticket: mika#1024
type: feat
module: mika-agent
tags: [prompt-assembly, compaction, summarizer, context-channel-leakage]
parent: mika#1009
sibling: [mika#1019, mika#1022]
---

# Plan: Summarizer Content Reform — Factual Assertions Over Conversational Language (mika#1009 Axis 2)

## Problem

mika#1009's finding doc (compounding factor 1) identified that the conversational summary uses bullet-point conversational language — "Discussed ticket #668 and agreed on disposition ITERATE" — that the LLM misreads as prior turns it participated in. Even when Axis 4 (load-prevention via `[context.summary].inject = false`) and Axis 3 (silent-mode budget cap via `[context.summary].max_tokens`) are not engaged, the surviving summary content continues to read as conversation in mode-conditional cases. **The structural fix for the leak class is to change the content shape itself: produce factual state assertions that read as records, not as turns.**

Concrete leakage example (paraphrased from #1009 evidence):

> _Pre-fix:_ `Discussed migration plan for #668 and agreed on disposition ITERATE; user wanted clarification on implementation steps.`
>
> _Post-fix:_ `Fact: Ticket #668 reviewed. Decision: disposition ITERATE. Outcome: implementation steps clarified.`

The post-fix shape preserves the same information but eliminates "discussed/agreed/wanted" verbs and any first-person framing. A future session reads it as historical record, not as conversation it participated in.

## Design

The compaction summarizer's behavior is governed by a single constant:

```rust
// crates/mika-agent/src/compaction.rs:14
const SUMMARIZATION_SYSTEM_PROMPT: &str = "\
You are summarizing a conversation between an AI executive assistant and their user.
Preserve: key decisions, action items, commitments, user preferences, important facts about people.
Discard: pleasantries, small talk, repeated information.
Keep the summary concise (under 500 tokens). Use bullet points.
If there is an existing summary, merge it with the new information.";
```

This is the entire knob. Reforming this string changes summarizer behavior for every agent (subject to per-agent Axis 4 opt-out and Axis 3 cap).

### Reformed prompt (proposed)

```rust
const SUMMARIZATION_SYSTEM_PROMPT: &str = "\
You are producing a factual record of what HAPPENED in a session, for a future session to read as history.
The output is a record FOR a future agent, NOT a record OF a conversation. Future readers did not participate in this session.

Format every bullet as a state assertion with one of these prefixes:
- `Fact:` for objective state (entities, references, timestamps, quantities)
- `Decision:` for choices made and disposition
- `Outcome:` for results and state transitions
- `Open:` for unresolved questions or pending work

Do NOT use:
- First-person language (we, our, I) or second-person (you, your)
- Conversational verbs that imply participation (discussed, agreed, decided together, wanted, asked)
- Process narration (then we, after that, next)

Do:
- Preserve key decisions, action items, commitments, user preferences, important facts about people
- Discard pleasantries, small talk, repeated information
- Keep the record concise (under 500 tokens) and use bullet points

If there is an existing record, merge new factual state into it; do not preserve conversational shape from the prior record.";
```

### Why this shape

**Why a forcing function on prefixes (`Fact:`/`Decision:`/`Outcome:`/`Open:`)**: prompts that say "use factual language" without a structural enforcer regress under load — the LLM falls back to its trained conversational default. Naming the four prefixes gives the model a checklist; it has to pick one per bullet, which structurally evicts narrative shape. Choice of four (not three, not five): `Fact`/`Decision`/`Outcome` cover the corpus per #1009's finding ("decisions, action items, commitments, preferences, facts"); `Open` adds the "pending question" class that #991's milestone-callback work demonstrated agents need to surface. The prompt does not enforce these via parser — the LLM reliably emits them when the prompt names them, and even partial compliance (a few bullets without prefixes) still degrades less than the current shape.

**Why explicit "NOT a conversation" framing**: the existing prompt opens "summarizing a conversation between..." which primes conversational shape. Reframing the meta-task ("producing a factual record of what HAPPENED") flips the prior. The "Future readers did not participate" sentence is the load-bearing one — it directly contradicts the leak's mechanism (LLM treats summary as participated history).

**Why explicit don't-list**: prompts that specify positive shape without negative shape leak the negative shape. "Use factual language" doesn't tell the LLM to NOT say "we discussed." Explicit `Do NOT use: First-person language; Conversational verbs; Process narration` closes the gap. The negative list is short by design — three categories with two-or-three examples each — to stay scannable.

**Why preserve all other behavior**: the existing prompt's "preserve/discard" lines + 500-token budget + bullet-point format + existing-summary merge all carry forward unchanged. This change is content-shape only; the structural envelope (length, bullet format, merge semantics) is not in scope.

### Rejected alternatives

- **Add a post-summarizer rewriter step** (run summary through second LLM call to "convert to factual"). Doubles cost. Adds a failure mode (rewriter can hallucinate). The summarizer can produce the right shape directly with a better prompt; that's the test #1009 implies.
- **Switch summarizer to a JSON schema** (output `{"facts": [...], "decisions": [...]}`). Forces a parser layer in the agent code (or downstream LLM has to read JSON-as-prose, which is its own leakage). The plain-text bullet format is what the consuming LLM actually reads; matching it at the producer is simpler.
- **Move the prompt to a skill** (`crates/mika-agent/src/skills/bundled/compaction/system_prompt.md`). The compaction call is engine-internal (not a tool the agent invokes), and the skills system exists for tool/context plumbing. Wiring summarization through skills would require non-trivial plumbing for zero benefit. Keep it as a `const &str` in the compaction module.
- **Per-agent summarizer prompt customization** (agent identity.toml carries an override). Pre-emptive scope creep. Today every agent shares one prompt. If a specific agent regresses post-#1024, file then.
- **Banned-word lint** in tests asserting summaries don't contain `discussed`, `agreed`, etc. Rejected because the prompt is the producer, not the artifact — the right test asserts the *prompt content* contains the forcing function language and the integration test asserts a sample summary uses prefix bullets. A banned-word lint would either false-positive (summary content quoting the user verbatim contains "we") or be too loose to catch the regression class.

## Implementation Steps

### Step 1: Replace the `SUMMARIZATION_SYSTEM_PROMPT` constant

**File:** `crates/mika-agent/src/compaction.rs`

Replace the existing 5-line constant at line 14 with the reformed prompt above. No other code changes — the constant is consumed by `summarize_messages()` at `compaction.rs:127` and threads through `LlmRequest.system` unchanged.

**Rationale:** Single point of edit. The function signature, message formatting (`## Existing Summary` / `## Messages to Summarize` headers in the user prompt), and `MAX_SUMMARY_CHARS` truncation are all preserved.

### Step 2: Update existing test fixtures

**File:** `crates/mika-agent/src/compaction.rs` (test module)

The existing tests at `compaction.rs:242` and `:281` exercise the compaction path with `MockLlmProvider`-supplied summary text. They assert on whether `replace_with_summary` was called and the resulting `load_conversation_summary` returns the expected content — they do NOT assert on prompt content shape. **No changes needed to these tests** — mock summaries are operator-supplied strings, orthogonal to prompt content.

### Step 3: Add prompt-content invariant test

**File:** `crates/mika-agent/src/compaction.rs` (test module)

Add a unit test asserting the `SUMMARIZATION_SYSTEM_PROMPT` constant contains the load-bearing forcing-function strings:

```rust
#[test]
fn summarization_prompt_enforces_factual_shape() {
    // Forcing function: prefix vocabulary
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("`Fact:`"));
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("`Decision:`"));
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("`Outcome:`"));
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("`Open:`"));

    // Anti-conversational framing
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("NOT a record OF a conversation"));
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("did not participate"));

    // Negative list
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("Do NOT use"));
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("First-person"));
    assert!(SUMMARIZATION_SYSTEM_PROMPT.contains("Conversational verbs"));
}
```

**Why this test:** the prompt is the deliverable. Future maintainers refactoring `compaction.rs` could regress the prompt back toward its current shape; this test catches that. Asserting on key strings (not exact-match) keeps the test stable across copy edits.

### Step 4: Document the contract

**File:** `crates/mika-agent/CLAUDE.md`

Add a subsection under "Conversation Compaction & Rewind" (or near the existing summary documentation). Suggested placement:

```markdown
**Summarizer output contract (#1024):** The compaction summarizer produces *factual state assertions*, not conversational summaries. Output bullets use one of four prefixes: `Fact:` (objective state), `Decision:` (choices and disposition), `Outcome:` (results and state transitions), `Open:` (unresolved questions). The prompt explicitly forbids first-person language, conversational verbs (discussed/agreed/decided), and process narration. This shape is per mika#1009 finding (Axis 2 — content reform): the summary block is consumed by the next session as system-prompt context, and conversational shape there causes the LLM to misread it as prior turns it participated in. The prompt is a single `const &str` at `compaction.rs:14`; tests at `compaction.rs:`<line> assert prompt invariants.
```

### Step 5: No identity.toml changes

This change has no per-agent toggles. Every agent benefits from the reformed shape (subject to existing Axis 4 opt-out). `well_known_agents.rs` is not touched.

## Test Strategy

### Unit tests (engine)

1. **Prompt invariant test** (Step 3): assert load-bearing strings present in the constant.
2. **Existing compaction tests** at `compaction.rs:242` and `:281` continue to pass unchanged — they don't assert on prompt content shape.

### Integration / eval tests

`crates/mika-agent/tests/eval/grounding_regressions/` already contains scenarios for context-channel leakage (per CLAUDE.md). Add a scenario:

- **Scenario:** factual-assertion summary survives a callback turn without producing degenerate "prior turns" reference.
- **Setup:** seed a summary in the DB whose content is reformed-shape (Fact/Decision/Outcome/Open prefixes); trigger a callback turn against an agent with `[context.summary].inject = true` and no `max_tokens` cap; assert the agent's response does NOT contain "we discussed" / "as we agreed" / "in our prior conversation" / similar conversational-recall patterns (forbidden-word assertion list).
- **Negative control:** same setup with the *current* summary shape (conversational bullets); verify the regression-reproduction test fails (the response DOES contain conversational-recall patterns). This is the frozen-fixture pattern per `tests/eval/grounding_regressions/README.md`.

The scenario validates the *consuming* side of the contract (the next-session LLM reads factual-shape correctly). The unit test validates the *producing* side (the prompt forces the shape). Together they close the loop; neither alone is sufficient.

### No real-LLM eval needed for unit gating

The unit tests use `MockLlmProvider`. The integration scenario can run on `MockLlmProvider` as well — the assertion is on the *agent's* response to a fixture-supplied summary, not on the summarizer's actual output. Real-LLM evaluation of the summarizer's output (does it actually emit Fact/Decision/Outcome bullets when run against real content?) belongs in deploy smoke per the runbook §3 and in calibration runs (`MIKA_EVAL_REAL_PROVIDERS`).

## Acceptance Criteria

Mirroring the ticket body for traceability:

- **AC#1**: `SUMMARIZATION_SYSTEM_PROMPT` in `crates/mika-agent/src/compaction.rs` is updated to enforce factual-assertion shape. No conversational verbs in the prompt; explicit "NOT a conversation" framing; named prefix vocabulary (`Fact:`/`Decision:`/`Outcome:`/`Open:`); explicit Do-NOT-use list (first-person, conversational verbs, process narration).
- **AC#2**: A unit test (Step 3) asserts the load-bearing forcing-function strings are present in the prompt constant. Test stable across copy edits (asserts on key substrings, not exact-match).
- **AC#3**: Existing compaction tests pass unchanged. No regression in compaction call path, message formatting, or summary persistence.
- **AC#4**: `crates/mika-agent/CLAUDE.md` documents the summarizer output contract under the Compaction subsection (Step 4), naming the four prefixes and the file:line citation.
- **AC#5**: An integration scenario in `tests/eval/grounding_regressions/` (Step 6 of test strategy) validates that a reformed-shape summary fed to a callback turn does NOT trigger conversational-recall patterns in the agent's response. Frozen-fixture negative control demonstrates the regression class is the conversational shape, not the summary content per se.

## Risks & Open Questions

- **R1 (low):** Behavior under non-Anthropic providers. Different LLMs respond differently to "Do NOT use" lists. Anthropic Sonnet handles negative-list prompts well; some open-source models (kimi-k2.5, deepseek-v3) sometimes invert them. Mitigation: the prompt's positive forcing function (named prefixes) does most of the work; the negative list is reinforcement. If a specific provider regresses post-deploy, surface in the deploy smoke per runbook §3 and consider per-provider variants. Out of scope for this PR.
- **R2 (low):** Existing summaries in the DB are conversational-shape. They will gradually be replaced by reformed-shape summaries via natural compaction (each agent that crosses the 50-message threshold rewrites its summary, the merge step flows new content into the existing). No migration is needed — the leak protection compounds over time. **Operator option:** if an agent has a particularly conversational summary causing observable degeneracy, manual `DELETE FROM messages WHERE role = 'system' AND content_type = 'summary' AND agent_id = ?` will force a fresh summarization on next compaction. Out of scope for this PR.
- **R3 (low):** The prompt's "under 500 tokens" budget is a soft bound the LLM can ignore; `MAX_SUMMARY_CHARS = 4000` (compaction.rs:11) is the structural cap. The reformed prompt with prefix vocabulary is slightly longer per bullet (prefix + content vs. content only), so summaries may approach the 4000-char cap more often. Monitoring: if `truncating oversized summary` warnings increase post-deploy, that's the signal to revisit. The truncation already cuts at a multi-byte char boundary safely.
- **R4 (low):** Composability with Axis 3's silent-mode truncation. Axis 3's `truncate_to_token_budget` cuts mid-content with a marker. If the cut happens between a `Fact:` prefix and its content, the result is `... Fact:` followed by the truncation marker — slightly awkward but readable. Acceptable; the alternative (boundary-aware truncation) is YAGNI for the silent-mode policy whose typical budget (1000 tokens, ~4000 chars) is at the structural cap anyway.
- **OQ1 (groom):** Should the prefix vocabulary be `Fact:`/`Decision:`/`Outcome:`/`Open:` (proposed, four options) or a shorter set? Two-option (`Fact:`/`Decision:`) is simpler but may force the LLM to file `Outcome` content under one of the others, regressing toward narrative. Architect: confirm or counter-propose.
- **OQ2 (groom):** Should the prompt be wrapped in `<context type="summary_prompt" trust="system">` tags or similar to signal "this is a system-level instruction, not user content"? Probably not (the prompt is in the `system` role, which already carries that signal), but flagging.

## Out of scope (deferred to Axis 1)

- **Axis 1**: anti-conversational `<context>` wrapper at the *injection* site (rather than producer site). Per #1009 plan: likely skippable. Revisit if Axis 4+3+2 in combination don't fully resolve the observed leak class.

## Sources

- mika#1009 finding doc: `mika/docs/solutions/best-practices/mika-arch-init-context-leakage-2026-05-06.md` (compounding factor 1: conversational summary language)
- mika#1019 (Axis 4 sibling, shipped 2026-05-07T17:39Z) — load-prevention via `[context.summary].inject = false`
- mika#1022 (Axis 3 sibling, shipped 2026-05-08T00:18Z) — silent-mode budget cap via `[context.summary].max_tokens`
- `crates/mika-agent/src/compaction.rs:14` (current `SUMMARIZATION_SYSTEM_PROMPT` constant)
- `crates/mika-agent/src/compaction.rs:127` (consumption site in `summarize_messages()`)
- `crates/mika-agent/CLAUDE.md` — Compaction subsection (documentation target)
- `tests/eval/grounding_regressions/README.md` — frozen-fixture pattern + tag vocabulary
- 2026-05-08 orchestrator handoff carry-forward (mika#1009 axes 2/1 sequencing)
