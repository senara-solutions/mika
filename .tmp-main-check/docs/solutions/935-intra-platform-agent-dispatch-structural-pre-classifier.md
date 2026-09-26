---
module: mika-agent/server
tags: [permission-policy, security, pre-classifier, agent-dispatch, relay]
problem_type: permission_bypass
category: architecture-patterns
---

# Intra-Platform Agent Dispatch Structural Pre-Classifier

## Problem

`mika ask --agent <peer>` invocations between platform agents (mika-arch, mika-dev, mika-qa) were being denied by mika-relay's permission-policy LLM classifier (Haiku-tier). The classifier evaluated raw Bash command strings and could not reliably distinguish `mika ask --agent mika-arch "<2KB brief>"` from arbitrary Bash — the trailing message payload defeated literal pattern matching.

Empirically proven across five canary rounds: any `mika ask` invocation with a real message argument was denied, while `--help` was allowed. The `...` in the prompt's TIER 1 allow-list was interpreted literally by the LLM.

## Solution

A structural pre-classifier in Rust (`crates/mika-agent/src/server/permission_pre_classifier.rs`) that runs BEFORE the LLM classifier. It recognizes the `mika ask --agent <known-peer>` dispatch pattern deterministically and returns `{"action":"allow"}` without LLM consultation.

### Key Design Decisions

1. **Allow-list over deny-list for compound commands.** Initial implementation used `any()` (allow if any sub-command matches). Review identified that this allows arbitrary sub-commands alongside a valid dispatch. Fixed to require ALL sub-commands to be recognized safe (either a valid dispatch or a known-safe prefix like `cd`/`pwd`).

2. **Command substitution rejection (quote-aware as of mika#938).** Any command containing `$(` or backticks **outside quoted regions** is rejected — those positions would trigger shell expansion at execution. Literal occurrences inside `"..."` or `'...'` are allowed: legitimate `mika ask` briefs frequently contain backticks (markdown inline code spans) inside the quoted message argument. The original blanket-reject heuristic in PR #937 caused a canary-v7 false-positive on the canonical `/mika-ask-arch` form; mika#938 refined Branch 5 to a quote-aware byte scanner (`contains_unquoted_metacharacter`). POSIX semantics: backslash escapes inside `"..."`, but is literal inside `'...'`.

3. **Safe pipe targets.** Commands piped to safe output formatters (`tail`, `head`, `grep`, `wc`) are allowed. Unknown pipe targets cause fallback to LLM.

4. **Static peer list.** The `INTRA_PLATFORM_DISPATCH_PEERS` const in `well_known_agents.rs` is the single source of truth. No config files, no dynamic discovery.

5. **Two hook sites.** The pre-classifier is hooked in both the HTTP server path (`handlers.rs`) and the CLI path (`ask.rs`), since the relay is invoked via `mika --agent mika-relay ask -` (CLI subprocess).

### Security Model

- TIER 3 patterns checked on the full command string (catches dangerous patterns even inside quotes)
- Command substitution (`$(`, backticks) rejected when **outside quoted regions** (mika#938 quote-aware refinement); literal occurrences inside `"..."` or `'...'` are allowed as message content
- Compound commands require ALL sub-commands to be on safe list
- Pipe targets must be from safe output commands list
- Only fires for `agent_id == "mika-relay"` with `[claude-pilot] ` prefix
- Only Bash tool type; other tools fall through

### False Positive: TIER 3 in message content

A message like `mika ask --agent mika-arch "explain rm -rf"` will trip the TIER 3 check because `contains_tier3_pattern` does substring matching on the full command string. This causes a false-negative (falls through to LLM) rather than a security hole. Accepted trade-off: the LLM path handles these correctly.

## How to Add a New Platform Agent

1. Add to `INTRA_PLATFORM_DISPATCH_PEERS` in `crates/mika-agent/src/well_known_agents.rs`
2. Add to `INTRA_PLATFORM_AGENTS` in `claude-pilot-py/src/claude_pilot/tier1.py`
3. Deploy both repos

## References

- Origin issue: mika#935
- Quote-aware refinement: mika#938 (Branch 5 false-positive on markdown briefs)
- Architecture doc: `docs/architecture/platform-internal-agent-dispatch.md`
- Plan: `docs/plans/2026-05-02-003-fix-skills-mika-ask-intra-platform-agent-plan.md`
- Pre-classifier source: `crates/mika-agent/src/server/permission_pre_classifier.rs`
