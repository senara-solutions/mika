---
title: "Merge two-step LLM tool contracts into single atomic tools"
category: architecture-patterns
date: 2026-04-07
tags: [skills, builtin-handlers, llm-tool-design, yagni, agent-native]
issue: senara-solutions/mika#477
related:
  - docs/solutions/architecture-patterns/harden-write-skill-variant-no-path-input.md
  - docs/solutions/architecture-patterns/skill-llm-override-db-layer-and-linked-unblock.md
---

# Merge two-step LLM tool contracts into single atomic tools

## Problem

The `skill-review` workflow was broken end-to-end for any agent without orchestrator-tier permissions. The two-step contract was:

1. Agent calls `review_skill` → returns `next_action: "Call write_skill_variant with skill_name and content..."`
2. Agent calls `write_skill_variant` → persists the variant under `generated/<provider>/<sanitized_model>/system_prompt.md`

In production, **step 2 never happened**. Two turn audits on 2026-04-07 (`mika-qa` session `68ce6546…` and `mika-dev` session `93bb81be…`) showed both agents reporting *"`write_skill_variant` is a built-in function that I do not have direct access to"*. They fell back to `write_agent_file`, which the sandbox rejected (`"Path resolves outside the base directory"` for `mika-qa`; `"Only orchestrator agents can access other agents"` for `mika-dev`). No variants were ever written. `~/.mika/agents/mika-dev/skills/self-dev/` had no `generated/` directory at all; `audit_events` had zero recent skill-write entries; `tool_calls` had zero `write_skill_variant` invocations in April.

## Root cause — *the leaky two-step contract*

The bug was not really in tool registration. It was in the **shape of the LLM contract**. Splitting one logical operation across two tools introduced three independent failure modes that no amount of registration tweaking could close:

1. **Surface drift.** `write_skill_variant` was a `KNOWN_BUILTIN`, but it was *not* declared in the skill-review skill's `tools.json`. The skill's system prompt told the agent to call it anyway. Whether the runtime registry exposed it to the LLM at all was a flaky function of which skills were active and which agent persona was running. The contract said "two tools"; the surface delivered only one.
2. **Instruction-as-protocol.** The whole second step was carried by a prose `next_action` string in the response payload. The LLM was free to ignore it, paraphrase it, "complete the task" with any plausible-looking alternative tool, or fabricate success. There was no machine-checkable contract — the protocol lived in English instructions inside JSON.
3. **No independent caller.** A grep of the entire codebase confirmed `write_skill_variant` had **zero callers outside the response string of `review_skill`**. It existed only as the second leg of a workflow that the LLM was supposed to chain. That is the textbook YAGNI signature: a separate primitive justified by a single chained use case.

These three together produce a fragile failure mode where the runtime, the prompt, and the LLM all *think* they did their part, and nothing gets written.

## Solution

Merge the two operations into one atomic tool. `review_skill` now accepts an optional `content` parameter:

- `content = None` → inspect-only (returns `root_prompt`, `tools_json`, `runtime_provider`, `runtime_model`, `existing_variant`, `linked`)
- `content = Some(...)` → inspect + persist atomically; same response plus `written`, `written_path`, `content_bytes`

`write_skill_variant` is deleted entirely: removed from `KNOWN_BUILTINS`, the `execute()` dispatch, every test, the skill template, and CLI/source comments. Path is still computed entirely from `ctx.provider_name` / `ctx.model_name` — there is no path input the agent can fabricate. All four prior safety guards (200 KB content cap, 50% truncation guard, overwrite-requires-force, linked-skill warning) are preserved verbatim, just inlined into the merged handler.

The skill-review system prompt is rewritten to teach the one-call workflow: *"Call `review_skill` with no `content` to inspect, then call `review_skill` with `content` to persist. Do not call `write_agent_file` for variant writes."* No more `next_action` instruction string anywhere in the response.

### Key code shape

```rust
// crates/mika-agent/src/skills/builtin_handlers.rs
async fn review_skill(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    // ... validate skill_name (length cap, traversal, null bytes) ONCE at the entry ...

    let content = match input.get("content") { /* Option<&str>, ≤ MAX_PAYLOAD_BYTES */ };

    if skill_name == "*" {
        if content.is_some() {
            return ToolOutput::error("'content' is not supported in batch mode.");
        }
        return review_skill_batch(...).await;
    }

    review_skill_single(
        &skills_dir, skill_name,
        canonical_provider, canonical_model, &sanitized_model,
        dry_run, force, content, ctx.skills_dirty,
    ).await
}

async fn review_skill_single(..., content: Option<&str>, skills_dirty: &AtomicBool) -> ToolOutput {
    // ... read root_prompt, tools_json, detect linked, compute variant_path ...

    if let Some(body) = content {
        // truncation guard, overwrite guard, then create_dir_all + write + skills_dirty.store(true)
        return ToolOutput::success(/* inspect fields + written/written_path/content_bytes */);
    }

    // inspect-only response
    ToolOutput::success(/* inspect fields + "written": false */)
}
```

## How to detect this anti-pattern

A two-step LLM tool contract is fragile when *all three* of these are true:

1. **Single-caller chain.** Tool B has no production caller other than tool A's response telling the LLM to call it. (`grep -RIn '\bwrite_skill_variant\b' crates/ tests/` returned only the instruction string and tests of B itself. No calling code.)
2. **Text-only handoff.** The second step is communicated to the LLM via a `next_action` / `next_step` / "now call X" string in tool A's response, not a structural mechanism (e.g., a follow-up tool call, a scheduled job, a callback).
3. **Separate registration surface.** Tool B can be missing from the agent's tool surface for reasons that have nothing to do with whether tool A was reachable — different skill, different permission tier, different `tools.json`, different always-on filter.

Any one of these is a smell. All three together is the same bug class as ours. Audit signal: search `tool_calls` for tool A's invocations, then check whether tool B's invocations exist in the same session within ~5 turns. If A is called and B is not, the chain is broken in production — even if the test suite is green.

## Prevention

- **One LLM-visible tool per atomic operation.** If the operation needs internal helpers, those are private functions, not tools.
- **No `next_action` strings.** If a tool result documents a required next call in prose, treat that as a code smell. Either the next call is mandatory (then merge it in) or it's optional (then teach it in the skill prompt, not the runtime payload).
- **Audit the call chain in production, not just tests.** A regression test that calls A then calls B passes — but it does not prove that the LLM, given A's response, will reliably emit B. The only honest signal is `tool_calls` in `mika.db` from real sessions.
- **Match the tool surface to the prompt surface.** Whatever the skill template tells the LLM to call, the skill's `tools.json` (and whatever default-tools filter applies) must expose. Drift between these is silent and only manifests for the agent personas that get bitten.

## Verification

1. Build, test, lint:
   ```bash
   cargo build -p mika-agent
   cargo test -p mika-agent skills::builtin_handlers   # 99 tests, all pass
   cargo clippy -p mika-agent --all-targets -- -D warnings
   ```
2. Symbol absence:
   ```bash
   grep -RIn '\bwrite_skill_variant\b' crates/ templates/    # only test names + comments
   ```
3. End-to-end:
   ```bash
   make deploy
   mika ask --agent mika-dev "use skill-review to review skill build-mika and write the variant"
   ls ~/.mika/agents/mika-dev/skills/build-mika/generated/deepseek/deepseek-v3-2/system_prompt.md
   ```
   Expect a single `review_skill` invocation in `tool_calls` and the variant file present on disk.

## Related

- [Hardening write_skill_variant — no path input](harden-write-skill-variant-no-path-input.md) — the prior PR that made write_skill_variant safe but kept it as a separate tool. This compound supersedes that design.
- [Per-skill LLM override and linked review unblock](skill-llm-override-db-layer-and-linked-unblock.md) — unblocked symlink writes that the merged tool now relies on.
