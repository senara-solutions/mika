---
title: Engine guards vs prompt rules for agent behavioral invariants
date: 2026-04-19
category: architecture-patterns
module: agent-core, self-dev, skills-engine
component: agent-behavior
problem_type: workflow_issue
tags: [as-above-so-below, fabrication, prompt-engineering, engine-hooks, structural-enforcement, gradient, orthogonality]
applies_when:
  - An agent's behavioral rule is added to a skill or core_memory and partially-but-not-fully holds
  - Multiple prompt iterations on the same class of bug each take 5-10 message round-trips and leave compliance gaps
  - The undesired behavior goes against the model's trained defaults (tool-use reflex, turn-closure reflex, fabrication under cognitive load)
  - Vincent or any engineer is tempted to "just tighten the prompt one more time"
---

# Engine guards vs prompt rules for agent behavioral invariants

## Context

On 2026-04-18 through 2026-04-19, a single long session produced a concrete demonstration of a pattern we had been vaguely aware of (and memory note `feedback_prompt_enforcement_fragile.md` hinted at): **prompt-level rules against trained model gradients partially work and drift under cognitive load; structural engine-level guards bind deterministically**.

Concretely — mika-dev (kimi-k2.5) was observed repeatedly failing three classes of behavioral invariant:

- **Fabrication** — producing confident-looking structured output (line numbers, PR numbers, milestone child lists, test summaries) without having actually observed the data. Seen on session `4cbc6de7` (milestone #12 dispatch), `749345f9` (duplicate child work items), `6ee2bebf` (self_model line-number hallucination), `098a579a` (deploy-verification test summary fabrication), and the #648 callback-handling "Milestone #7 complete" summary with wrong children.
- **Wrong-tool-reach** — calling `read_agent_file` to try to read `core_memory` sections that are already auto-injected into the system prompt. Kimi's trained default is "to see X, call a tool for X."
- **Non-persistence** — ending a turn with substantive conclusions as text only, without calling `store_fact` to preserve the institutional knowledge for future sessions.

Over ~12 hours of session work, **each class was addressed at the prompt layer at least twice**. Every fix partially worked. Every fix drifted. The full arc:

1. Hot-patch `soul.md` with an Operational Memory Rule → soul.md is a seed file, changes don't propagate to `core_memory.self_model`. Fix ineffective.
2. Add a strengthened `Fabrication risk` block to `core_memory.self_model` → fires on some incidents, rationalized past on others.
3. Expand Rule 4 in self-dev to cover `run_gh` input-shape discipline → she reads the shorthand literally anyway on the first call; engine wrapper's explicit error recovers her the second call.
4. Add `Operational memory` block to `core_memory.self_model` → mika-qa (different skill, same pattern) complies reliably; mika-dev still misses sometimes.

Then the engine fixes landed:

- **mika#645** — `read_agent_file` now rejects `core_memory/*` paths with a domain-specific error pointing to where the data actually lives. **Fired correctly on first trigger.** No prompt discipline required.
- **mika#648** — turn-end persistence evaluation hook: when the agent produces diagnostic-shaped output without any write-tool call, the engine surfaces a check before `EndTurn`. Ships deterministic enforcement.

The contrast is the data point. Prompt iteration: 12 hours, 5-10 messages per fix, residual gaps. Engine fix: one iteration, deterministic, covers the whole class.

## Guidance

**Classify agent behavioral failures by whether they go WITH or AGAINST the model's trained gradient.**

**With-gradient behaviors** hold reliably at the prompt layer:
- Identity ("I am the orchestrator, I don't implement")
- Voice / style (terse, issue-prefixed, no filler)
- Domain vocabulary (use `task` not `work_item`)
- Format-of-response (structured markdown sections, specific tag usage)

These all go WITH what LLMs are trained to do — hold stable persona voice, follow stylistic norms. Prompt rules are the right layer.

**Against-gradient behaviors** need engine-level structural enforcement:
- Overriding a tool-use reflex (kimi wants to call a tool to read data that's already in context)
- Overriding a turn-closure reflex (LLMs are trained to close turns with text responses, not write-tool calls)
- Overriding a completion-pattern reflex (if the user prompt shape implies a structured answer, the model will produce that structure regardless of whether it has the data)
- Any invariant phrased as "when X happens, do NOT do Y" where Y is the model's trained default response to X

When a bug falls in the against-gradient category, **reach for an engine hook, not a prompt patch**. The cost pattern makes the classification visible: if 2-3 prompt iterations don't converge, the problem isn't the rule — it's the layer.

**Sequencing rule — "as above, so below":** the above (engine) must be in place before the below (prompt scaffolding) simplifies. Removing memory-side scaffolding before the engine fix ships creates a regression window. Keep the prompt rule as bridge scaffolding until the engine hook deploys, then remove it in a follow-up.

## Why This Matters

### The cost argument

Prompt iteration on against-gradient bugs has a measurable cost pattern:

| Dimension | Prompt-level fix | Engine-level fix |
|---|---|---|
| Round-trips per attempt | 5-10 messages | 1 implementation |
| Attempts until convergence | 2-3 (with residual gaps) | 1 (deterministic) |
| Reliability under cognitive load | Partial, drifts | Structural, binds |
| Cost (tokens + developer time) | High and recurring | One-shot |
| Observability | Invisible when it doesn't fire | Logged/metric on every trigger |

When prompt iteration cost exceeds engine-change cost for the same class of bug, **the classification itself is wrong**. This is the signal to reach for a Rust change, not another prompt edit.

### The orthogonality argument

A rule that lives in two places (prompt AND memory AND skill) drifts as each copy is edited independently. A rule enforced at the engine layer has one source of truth and one failure mode. This is orthogonality applied to behavioral invariants: put the rule at the layer that can enforce it.

### The observability argument

Prompt rules fail silently — when the rule doesn't fire, there's no log, no metric, just an incident later. Engine guards fail loudly with structured errors and can have metrics attached (see mika#645's domain-specific rejection message, and mika#648's turn-end check).

## When to Apply

Apply engine-level enforcement when:

1. **Prompt iteration cost has exceeded engine-change cost.** Use the cost pattern (5-10 messages per attempt, 2-3 attempts, residual gaps) as the threshold. If you're about to iterate a third time on the same class, stop and file an engine ticket.
2. **The behavior fights a trained model default.** Tool-use reflex, turn-closure reflex, fabrication-under-load reflex, completion-shape reflex. These are against the gradient; prompts won't fully hold.
3. **You want deterministic, observable enforcement.** Engine guards log. Prompt rules fail silently.
4. **The rule is structural, not behavioral.** "core_memory is auto-injected, no tool needed" is a fact about the system architecture, not a behavior of the agent. Structural facts belong in engine code (tool descriptions, system prompt preambles, tool-handler rejections), never in memory content.

Do NOT reach for engine enforcement when:

1. The rule goes WITH the gradient (identity, voice, domain vocabulary). Prompts work fine.
2. The rule is agent-specific — what qualifies as a failure mode varies per-agent (mika-dev vs mika-qa vs a customer agent). Memory is correct for agent-scoped learnings.
3. You haven't tried the prompt-layer fix yet. One attempt is cheap. Two is evidence. Three is the threshold.

## Examples

### Before — prompt-only fix for fabrication (failed to bind)

```markdown
# In core_memory.self_model:
**Fabrication risk:** After tool failure, one follow-up tool call or status
update — no narrative close-out without evidence. If no recovery path, stop
and report.
```

Result on session `098a579a`: read_agent_file failed, she STILL produced a confident structured answer (citing fabricated line numbers like "line 7-8", "lines 3-5") ignoring the rule. The rule was in context and didn't fire.

### After — engine guard for the same class (bound immediately)

Implemented in mika#645:

```rust
// In crates/mika-agent/src/tools/read_agent_file.rs:
if is_core_memory_path(&path) {
    return Err(format!(
        "Path '{}' is not filesystem-accessible. core_memory sections \
         (including '{}') are auto-injected into your system prompt on \
         every turn. To modify core_memory, use update_core_memory.",
        path, section_name
    ));
}
```

Result on re-test: same session shape, she STILL called `read_agent_file("self_model.md")` on first try. Engine returned the domain-specific error. She read it, pivoted to reading from context, produced a non-fabricated answer. **The guard fired deterministically on first trigger. No prompt discipline required.**

### Before — prompt rule for persistence (partial compliance)

```markdown
# In core_memory.self_model:
**Operational memory:** Persistence IS the acknowledgment. When you reach
diagnostic conclusions, validate designs, or receive institutional knowledge
that future sessions should inherit, call store_fact BEFORE producing output
text. Never end a turn with "this validates X" or "this means Y" without
persisting it.
```

Result: mika-qa complied reliably (called `store_fact` after every PR review). mika-dev complied intermittently (missed the #648 callback-handling turn, missed multiple substantive diagnostic turns).

### After — engine turn-end hook (ships in mika#648)

```rust
// Sketch in crates/mika-agent/src/agent.rs EndTurn guard chain:
if detect_informational_input(&user_msg).is_some()
   || detect_persistable_output(&assistant_text).is_some()
{
    let wrote = tools_called.iter().any(|t| PERSISTENCE_WRITE_TOOLS.contains(t.name));
    if !wrote {
        return Err(Guard::PersistenceCheck {
            message: "This turn appears to contain institutional knowledge \
                      but no persistence was called. Proceed with EndTurn \
                      or call store_fact first?"
        });
    }
}
```

Result: ships deterministic enforcement. Model gets a synthetic prompt before `EndTurn` closes; must explicitly decide to persist or close. Gap is closed structurally.

## Prevention

1. **When a behavioral fix feels like it needs a prompt rule, first ask: does this go WITH or AGAINST the trained gradient?** Identity/voice/style → prompt. Against a trained default → engine.
2. **Track prompt-iteration cost as a classification signal.** The third attempt to fix the same class of bug via prompt is the threshold to stop and file an engine ticket.
3. **When you do ship a prompt-level rule as bridge scaffolding, file the engine ticket at the same time.** The scaffolding is temporary; the engine fix is the durable home.
4. **In the engine ticket, include the prompt-iteration log as motivation.** This prevents YAGNI objections — the engine guard is "earned by observed failure modes" rather than speculative infrastructure.
5. **Apply the correspondence principle: `as above, so below`.** The above (engine) holds the invariant; the below (memory/prompt) either mirrors it during the bridge window or simplifies once above is deployed. Don't preempt by removing the below before the above is live.

## Related

- mika#645 — structural guard against core_memory mis-access via `read_agent_file` (the first engine guard shipped on 2026-04-18 validating this pattern)
- mika#648 — engine-level turn-end persistence evaluation hook (second engine guard, same shape)
- mika#647 — engine pre-tool context-redundancy check (extends #645's pattern, sibling structural fix)
- mika#650 — `send_message` `chat_id == 0` handling (agent-side typed result instead of gateway 400 — similar pattern: structural fix at the right layer)
- Memory: `feedback_prompt_enforcement_fragile.md` (earlier hint of this principle — "Don't use prompt-level budgets/limits; LLMs rationalize crossing them. Use structural constraints.") (auto memory [claude])
- Memory: `project_skill_propagation_lock.md` (abandoned 2026-04-18 using the same reasoning — Claude + env var was sufficient; no DB schema needed) (auto memory [claude])
- Principle referenced throughout the 2026-04-18/04-19 session: "as above, so below" (hermetic correspondence) applied to software architecture

## One-line takeaway

**When the cost of prompt iteration exceeds the cost of an engine change for the same class of bug, the classification is wrong — move to the engine layer.**
