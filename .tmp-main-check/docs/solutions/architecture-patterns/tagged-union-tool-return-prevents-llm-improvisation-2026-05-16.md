---
module: agent-core
tags: [tools, prompt-contract, tagged-union, serde, pr-merge-gate, safety]
problem_type: architecture_pattern
category: architecture-patterns
created: 2026-05-16
ticket: mika#794
prior_art: prompt-vs-tool-contract-mismatch-2026-04-24.md
---

# Tagged-Union Tool Return Prevents LLM Improvisation

## Problem

When a tool returns unstructured string errors, the LLM has no documented branch
for the failure state and improvises a response. In production, this caused two
Rule 6 violations (mika#485, mika#792) where the agent called `run_gh pr merge`
as a "fallback" when `pr_merge_with_gate` returned an error string.

The root cause is a **prompt-vs-tool contract mismatch**: the tool can return
outcomes the prompt doesn't document, creating gaps where the LLM fills in with
unsafe actions.

## Solution

Replace unstructured string returns with a **serde-tagged enum** (`#[serde(tag = "action")]`)
where every outcome is a named variant the prompt can branch on exhaustively.

### Pattern: Tagged-Union Tool Contract

```rust
#[derive(Serialize)]
#[serde(tag = "action")]
pub(crate) enum ToolResult {
    #[serde(rename = "success")]
    Success { ... },
    #[serde(rename = "blocked")]
    Blocked { reason: BlockReason, detail: String },
    #[serde(rename = "errored")]
    Error { kind: ErrorKind, detail: String },
}
```

Key properties:
1. **Every outcome has a name** — the prompt branches on `action` field
2. **Sub-enums carry typed reasons** — `BlockReason`, `ErrorKind` use nested `#[serde(tag)]`
3. **Backward compatibility via additive fields** — existing variants retain old fields
4. **Errors return `ToolOutput::success()`** — the tool ran; the outcome is structured, not a crash

### Pattern: Preflight Before Action

Add a cheap read-only API call before the expensive mutating action to detect
impossible states early:

```rust
// Step 1: Preflight — detect impossible states
let preflight = run_gh_pr_view(pr, repo, token).await?;
if let Some(result) = classify_preflight(&preflight) {
    return Ok(ToolOutput::success(serialize(&result)?));
}

// Step 2: Proceed with action (only reached if preflight passed)
```

The preflight is a pure function (`classify_preflight`) — easily testable without
subprocess mocking.

### Pattern: Prompt Exhaustive Handling

The prompt documents every variant with explicit prohibitions on fallback:

```markdown
**`"blocked"`** — Branch on `reason`:
  **`merge_conflict`** — Do NOT call run_gh pr merge. Notify only.
  **`required_check_failed`** — Dispatch fix. Use failing_checks array.
  **Unrecognized `reason`** — Do NOT call run_gh pr merge. Notify only.

**`"gate_errored"`** — Do NOT fall back to run_gh pr merge.
```

The "unrecognized reason" catch-all prevents future improvisation when new variants
are added before the prompt is updated.

## Key Decisions

1. **Keep `run_gh_checks --required` for check classification** — preflight detects
   state issues (CONFLICTING, CLOSED, DRAFT); `gh pr checks --required` has
   server-side filtering that `statusCheckRollup` doesn't replicate. Different
   questions, different tools.

2. **`failing_checks` retained at top level** — backward compatibility for existing
   prompts that read `action == "blocked"` + `failing_checks[]`. The new `reason`
   field is additive.

3. **Errors use `ToolOutput::success()`** — the distinction is: `ToolOutput::error()`
   for caller bugs (bad input, missing token); structured `GateError` variant for
   "tool ran, infra failed." The prompt branches on `action` in both cases.

## Applicability

Use this pattern when:
- A tool can return multiple named outcomes
- Failure modes should trigger different prompt branches (not a single "error" path)
- The LLM might improvise unsafe actions when it encounters unstructured errors
- Backward compatibility with existing prompt branches is required

## Related

- `docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md` — the bug class this pattern fixes
- `docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md` — same principle applied to verdict handling
- mika#793 — follow-up to apply the same pattern to self-dev and self-dev-webhook-ci
