---
title: "mika-arch first dogfood — operational verification of the v1 architect agent"
date: 2026-04-25
category: best-practices
module: mika-arch
problem_type: best_practice
component: architect_agent
severity: medium
applies_when:
  - Verifying a newly-deployed Mika agent end-to-end after first provisioning
  - Operating mika-arch's groom-ticket + second-review loop on a real ticket
  - Calibrating expectations for skill-level model overrides vs agent base model
tags:
  - mika-arch
  - architect-agent
  - dogfood
  - milestone-validation
  - skill-model-override
  - smoke-test
---

# mika-arch first dogfood — operational verification of the v1 architect agent

## Context

mika-arch v1 (mika-platform#51 / mika#811) shipped 2026-04-25 in PR #813. The plan's Unit 5 was an explicit operational dogfood gate: run the agent end-to-end on a real small p2 ticket before declaring the architect-review loop production-ready. Skipping the dogfood would have meant relying on mika-arch in subsequent grooming sessions without ever having validated that both skills (groom-ticket on first pass, second-review on second pass) actually produce the expected output shape under real conditions.

The dogfood ran on **mika#814** — a small Rust config-loader bug (`MIKA_KG_DOCS_ROOTS` env var not parsing as `Vec<PathBuf>`), itself surfaced during the post-deploy smoke test moments earlier. Two interactive turns (first-pass + second-pass after refinements) over ~5 minutes.

## What worked

### 1. Provisioning came up cleanly once `kg_docs_roots` was supplied via TOML

Identity rendering, soul.md seeding, four-corpus `agent_kg_corpora` rows (mika / mika-platform / mika-skills / mika-cloud), lexical ingestion across all four roots — all fired correctly on the first restart after configuration was complete.

```
sqlite3 ~/.mika/data/mika.db "SELECT agent_id, COUNT(*) FROM agent_kg_corpora WHERE agent_id='mika-arch'"
mika-arch|4
```

### 2. Both skills work end-to-end

The groom-ticket → second-review loop completed successfully on the first attempt. Both produced structured, scoped output.

**First-pass (groom-ticket) response shape:**
- Annotated plan in a table format (Section / Assessment columns)
- Principle checklist (5 principles cross-checked: library-native behavior, single-file change, test grid coverage, explicit scoping, no schema/migration risk)
- Open questions tagged "non-blocking"
- Final disposition statement

**Second-pass (second-review) response shape:**
- Prior-findings tracking table (Prior Finding / Status — RESOLVED for both)
- Issue-state confirmation via `gh_read.issue_view` (proves the new builtin tool works in production)
- Final verdict statement

### 3. `--session-id` correlation works across two `mika ask` invocations

The plan's correlation primitive (reuse `mika ask --session-id <captured>` instead of inventing a new flag) was verified end-to-end. mika-arch read its own prior review from session memory; no payload re-pass needed in the package.

```
sqlite3 ~/.mika/data/mika.db "SELECT id FROM sessions WHERE agent_id='mika-arch' ORDER BY started_at DESC LIMIT 1"
f02d372e-3202-47fa-b095-2294fab147e4
```

### 4. `gh_read` builtin works

Second-review's response opened with: *"Issue state confirmed: OPEN, labels bug/p2-normal/agent-core/knowledge-graph. Issue body matches the brief's summary. No linked PR yet."* — that's the agent invoking `gh_read.issue_view 814` and reporting back. The four-op read-only allowlist held; no write attempts surfaced.

## What didn't work as planned

### A. Skills ran on Kimi K2.5, not Opus 4.7 / Sonnet 4.6

The mika-arch v1 plan specified per-skill model overrides:
- `mika-arch-groom-ticket` → Anthropic-direct `claude-opus-4-7`
- `mika-arch-second-review` → Anthropic-direct `claude-sonnet-4-6`

Verified actual model usage from `llm_calls`:

```sql
SELECT model FROM llm_calls WHERE session_id='f02d372e-3202-47fa-b095-2294fab147e4';
moonshotai/kimi-k2.5  -- 6 calls, all on Kimi
```

Cause: PR #813's committed plan explicitly scoped this out — "NOT in scope: `[llm]` section revival in skill.toml. Per-skill LLM overrides use the existing DB-backed `skill_overrides.llm_provider/llm_model` path."

But mika-arch follows D2 (no `skill_overrides` rows for well-known agents — identity allowlist instead). So there is **no place for the per-skill model override to live**. The skills fall through to mika-arch's agent base model (Kimi K2.5).

The skills produced architect-class output structure on Kimi K2.5 nonetheless — the prompts carried the discipline. But the explicit two-tier model strategy (Opus first-pass, Sonnet iteration-pass) the plan called for is not what's running. Cost is lower than budgeted; quality is unverified against the higher tier.

**Follow-up needed:** decide between (a) reviving `[llm]` section in skill.toml (architecturally cleanest — model lives with the skill), (b) extending identity `[skills].allowlist` entries to allow per-skill model overrides (keeps identity as single source of truth for well-known agents), or (c) accepting Kimi-as-base for v1 and moving the override decision to v2. File a separate ticket once the path is decided.

### B. First-pass disposition was "Proceed", not the literal "READY"

The skill prompt contract was:
> Output contract: annotated plan content as a string + an explicit Disposition line: `Disposition: READY` | `Disposition: ITERATE` | `Disposition: ESCALATE`.

First-pass actual output ended with:
> **Proceed.** The plan is clean, the scope is tight, and the risk surface is minimal. Dispatch to `claude-pilot` when ready.

Second-pass (correctly) ended with:
> ## Verdict: GROOMED

The second-pass adheres exactly. The first-pass took semantic liberty. Likely root cause: the system prompt allows the disposition keyword to appear anywhere in the response, and Kimi paraphrased to "Proceed" while the structural intent matched READY. Mild prompt-adherence drift; not blocking.

**Mitigation:** tighten the first-pass prompt's output contract — require the literal "Disposition: READY" / "Disposition: ITERATE" / "Disposition: ESCALATE" line as the final line of the response, not a paraphrase. Match what second-review did right.

## Numbers

| Metric | First-pass | Second-pass |
|---|---|---|
| LLM calls | 3 | 3 |
| Total input tokens | 23,109 | 31,000 |
| Total output tokens | 1,298 | 762 |
| Cache read tokens | 6,528 | 19,712 |
| Wall-clock | ~33s | ~13s |
| Model | moonshotai/kimi-k2.5 | moonshotai/kimi-k2.5 |
| Tool calls | 1 (initial scoping) | 1 (gh_read.issue_view) |

Total dollar cost on Kimi: pennies. Same workload on Opus 4.7 first-pass + Sonnet 4.6 second-pass would have been roughly 30-50× higher per call given cache-friendly volumes, but still well under \$1 for the round.

## Operational learnings

1. **The dogfood gate (Unit 5 of the mika-arch plan) was load-bearing.** Two real gaps were found before any subsequent dispatch relied on the agent. If we'd skipped Unit 5, we'd have shipped tickets through an architect that was running on the wrong model for an unknown stretch.
2. **The "GROOMED plan" comment on the issue is the contract.** mika-arch's second-pass output was suitable to paste directly as a comment on mika#814 with the implementation approach. /mika dispatch reads the issue body + comments; this gives mika-dev the architect-reviewed plan as the implementation contract without any extra workflow step.
3. **`gh_read` is the architect's eyes.** Second-review used `gh_read.issue_view` to verify the issue still matched the brief — a fact-check pattern that's only possible with the builtin tool. Without `gh_read`, the architect would be operating on the brief alone, vulnerable to the brief drifting from the issue.
4. **Architect-class prompts carry the discipline even on a smaller model.** Kimi K2.5 produced structured tables, principle checklists, and citation-style reviewing without any handholding. The skill prompts (mika-arch-groom-ticket, mika-arch-second-review) plus the review-guide reference were sufficient to deliver the operating shape. The model-override question is about *quality of judgment*, not *output structure*.
5. **First-deploy gotchas were config, not code.** The PR shipped clean. The two issues that bit on smoke-test (`MIKA_DEV_MODE` not set, `MIKA_KG_DOCS_ROOTS` env-var parsing) were both runtime configuration gaps, not implementation bugs. The fix in both cases was operator-side (`~/.mika/.env` and `~/.mika/config.toml`).

## Operator-proxy memory-seeding pattern (added 2026-04-26)

When mika-arch surfaces facts she'd want to persist but lacks the tool path to do so (today: `update_core_memory`/`store_fact`/`update_fact` denied via `MIKA_ARCH_DISABLED_TOOLS` — see mika#818 for the durable fix), the operator can act as proxy: ask mika-arch to return the writes in structured JSON form (5 core memory blocks + 4 fact categories with the contract shapes the tools would have used), then apply them directly to the `core_memory` / `people` / `preferences` / `commitments` / `events` tables scoped to `agent_id='mika-arch'`. UPSERT semantics mirror the tools: core_memory is `INSERT … ON CONFLICT(agent_id, key) DO UPDATE` (replace, not append); preferences is the same on `(agent_id, category)`; people uses canonical_name as the conflict key; events and commitments are append-with-IGNORE on the partial-unique constraints.

Validated 2026-04-26 on mika-arch session `83519e10-…`: 5 core memory blocks populated (~1122 tokens / 2500 budget), 1 person, 4 preferences, 3 events, 1 commitment seeded. Persists across mika-server restarts (data is in DB, not memory), so when the denylist fix lands and mika-arch resumes direct write capability, the seeded state stays as the baseline — future writes append rather than replace.

The pattern is **temporary scaffolding for a tool gap**, not steady-state. Once mika#818 lands and mika-arch can write directly, retire this pattern. Document it here because: (a) the pattern *worked* and may be reused for similar future gaps where an agent needs to persist state but the tool path is closed by design or by accident, and (b) the seeded JSON-shape contract (the structured response we asked mika-arch to return) is reusable as a "convert your useful state into DB-applicable form" pattern for any agent.

## Recommended follow-ups

- **Already filed:** mika#814 (env-var parse bug), mika#815 (D2 cross-cutting migration for mika-dev/mika-qa/mika-relay).
- **To file:** per-skill model override path for well-known agents (the skill.toml `[llm]` section question above). Block Path A (skill.toml `[llm]`) on architect alignment with the rulebook; this is a v2 feature unless Vincent calibrates the dogfood quality on Kimi as insufficient.
- **Prompt tweak (cheap):** tighten the groom-ticket prompt's output contract to require the literal "Disposition: <KEYWORD>" line as the final line. ~5-line edit to `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md`. Direct commit, doc-only-style edit, no /mika dispatch.

## Verdict for Unit 5

**Dogfood completed successfully.** mika-arch is operational. Both skills work end-to-end. Two non-blocking gaps found and documented. The architect agent is ready for use in subsequent grooming sessions, with the explicit caveat that current quality is calibrated to Kimi K2.5 — not to the Opus/Sonnet tier the plan envisioned.
