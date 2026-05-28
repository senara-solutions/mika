# Plan: perf(permission-policy): suppress haiku reasoning after JSON response

**Ticket:** mika issue#768
**Type:** bug/perf
**Scope:** prompt-only change to `skills/bundled/permission-policy/system_prompt.md`

## Problem

mika-relay (haiku, permission-policy skill) emits verbose reasoning paragraphs after its required JSON response. claude-pilot's transport parses the JSON correctly (via `extracted JSON from noisy stdout`), so this is a **cost/latency issue, not correctness**:

- ~350 wasted output tokens per permission check
- ~$0.28 and ~10 minutes wasted per full pilot run (~200 checks)
- Compounds across multiple pilot runs per day

## Root Cause

The system prompt tells the model WHAT to emit (`respond {"action": "allow"}`) but not that the JSON is the **ONLY** acceptable output. Haiku interprets "respond with this JSON" as "include this JSON in your response" and helpfully adds reasoning.

## Approach

Add an explicit output-format directive to the permission-policy system prompt. The ticket's proposed fix is well-specified and correct. The economic framing in the directive makes the constraint load-bearing for the model.

## Changes

### 1. `skills/bundled/permission-policy/system_prompt.md`

Add an **Output format (mandatory)** section immediately after the activation gate block (after line 8, before the `---` separator at line 9). Position rationale: placing the output constraint before the tier definitions ensures the model reads the format rule before encountering the tier-specific response shapes.

```markdown
## Output format (mandatory)

When the policy applies (message begins with `[claude-pilot] `), your ENTIRE
response is a single JSON object on a single line or pretty-printed — nothing
before, nothing after. No "Reasoning:" paragraph, no bullet lists, no preamble,
no trailing explanation.

- Correct: `{"action": "allow"}`
- Correct: `{"action": "answer", "answers": {"...": "..."}}`
- WRONG: `{"action": "allow"}\n\nReasoning: ...`
- WRONG: `Classifying as TIER 1.\n{"action": "allow"}`

The transport layer downstream parses your stdout as JSON. Extra text is
silently stripped but costs output tokens and latency on every permission
check — ~350 wasted tokens and ~2s wall-clock per check, which compounds to
~$0.28 and ~10 min per full claude-pilot pipeline run.
```

This is the exact directive from the ticket body with minimal formatting adjustments.

### 2. No other files changed

- No Rust code changes — the prompt is loaded at build time via `build.rs` and the `BUNDLED_SKILL_MANIFESTS` constant. Recompilation picks up the new prompt automatically.
- No `skill.toml` changes — the trigger keywords and metadata are unchanged.
- No test changes — there are no prompt-content tests for permission-policy. The integration test surface is the pre-classifier in `permission_pre_classifier.rs`, which is unaffected (it bypasses the LLM entirely for structural matches).

## Sibling context

The KG subject extractor (#876) already has a similar parse-tolerance mechanism (`extract_first_json_object()` brace-matching fallback) for haiku-class models that emit reasoning around JSON. That's the defensive parser side; this ticket is the offensive prompt-discipline side. Both are complementary.

## Verification

Per ticket ACs:
- [ ] `grep "extracted JSON from noisy" <pilot-log>` should return zero or near-zero hits on subsequent pilot runs
- [ ] Permission check latency drops to ~200–500ms (haiku structural minimum)
- [ ] Cost per pilot run drops from ~$0.28 to near-zero for permission reasoning waste

Verification requires a live pilot run after deploy — not automatable in CI.

## Out of scope

- Structured output / tool_use migration (separate, bigger ticket per #768)
- Per-provider/model prompt variants for permission-policy
- Changes to claude-pilot's noisy-stdout parser (it's a correct fallback)

## Risk

**Low.** Prompt-only change to a single skill. The directive is additive (no existing content removed). Worst case: haiku ignores the directive and continues emitting reasoning (status quo, no regression). The existing `extracted JSON from noisy stdout` parser in claude-pilot handles that case correctly.
