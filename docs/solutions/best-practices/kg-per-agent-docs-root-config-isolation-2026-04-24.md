---
module: kg
date: 2026-04-24
problem_type: best_practice
component: tooling
severity: high
tags:
  - knowledge-graph
  - per-agent-config
  - corpus-isolation
  - identity-toml
  - docs-root
applies_when:
  - Multiple agents share a server process but serve different knowledge domains
  - An agent ingests docs from a corpus unrelated to its purpose
  - KG extraction costs scale N× with agent count despite shared corpus
  - Operators need to disable KG for specific agents without affecting others
---

# Per-Agent KG docs_root Configuration — Stops Context Pollution from Wrong Corpus

## Context

Eleven mika agents shared one hardcoded `docs_root` (`mika/docs/solutions/`). Three of them — the odds-engine team — work on Polymarket trading strategies, not platform engineering. They ingested mika's engineering docs and built their KG from that corpus.

The failure mode is subtle: an odds-engine agent answers a trading question with confident coherence drawn from the wrong corpus. The KG has entries, the model has context, the reasoning looks structured — but the answer is contaminated by platform-engineering background rather than grounded in trading-strategy domain knowledge. **Coherence borrowed from the wrong corpus is anti-correctness masquerading as correctness.** This is harder to detect than "the answer is missing data" — the presence of irrelevant-but-coherent context masks the absence of relevant context.

This is a correctness bug, not a cost optimization. Cost reduction happens as a side effect of the shared-corpus model.

## Guidance

Each agent's `~/.mika/agents/<name>/identity.toml` gains a `[kg]` section:

```toml
[kg]
enabled = true                    # default: true
docs_root = "/absolute/path"      # optional; falls back to global chain
```

### Resolution Chain (per-agent → global → CWD-default)

1. **Per-agent:** `identity.toml` `[kg].docs_root` — highest priority
2. **Global env:** `MIKA_KG_DOCS_ROOT` environment variable
3. **Global config:** `settings.kg_docs_root` from config.toml
4. **CWD default:** `<CWD>/docs/solutions` — container-native fallback

### Behavior Matrix

| `enabled` | `docs_root` set | Behavior |
|-----------|-----------------|----------|
| `true` (default) | set | Validate path exists as directory; hard-error if not; use with computed `docs_root_hash` |
| `true` | unset | Fall back to global chain; hard-error on explicit env/config source if missing; warn-and-skip on CWD default |
| `false` | any | Skip KG entirely — no `LexicalIngestor`, `SubjectExtractor`, `SubjectEntityResolver` constructed |

### Implementation Pattern

The resolver (`resolve_per_agent_docs_root`) returns a typed enum:

```rust
pub enum KgAgentConfig {
    Disabled,
    Enabled { docs_root: PathBuf, docs_root_hash: String },
}
```

This eliminates partial states — either the agent has a validated path + hash, or KG is entirely disabled. The result is cached on `AgentState.kg_config` at init time.

### Hard-Error Policy

**Explicit paths that don't exist fail loud.** If an operator sets `docs_root = "/data/trading-docs"` and the path doesn't exist, the agent fails to start with a clear error. Silent fallback would produce exactly the contamination this feature prevents.

**CWD-based default uses warn-and-skip.** The container-friendly default at `<CWD>/docs/solutions` may not exist in every environment — that's expected, not a misconfiguration.

**Empty-string env/config is a distinct case.** `MIKA_KG_DOCS_ROOT=""` is treated as a configuration mistake, not a valid path. The resolver logs a distinct warn and disables KG for the agent — it does not hard-error (the operator may have set the env var to disable KG across the board).

### Shared-Corpus Semantics

Two agents with the same resolved `docs_root` → identical `docs_root_hash` → shared row set in the v27 shared-layer tables. Extraction cost drops from N× to 1×. The hash is 16-hex-char SHA-256 of `fs::canonicalize(path)`, stable per-host across restarts.

### `enabled=false` Does NOT Delete Existing Rows

Rows in the shared-corpus tables belong to the `docs_root_hash`, not the disabled agent. Disabling an agent prevents future writes but doesn't affect shared data. Cleanup requires `mika kg purge --agent <name>` (#779).

## Why This Matters

Cross-corpus contamination is a quality-primary correctness bug. The agent doesn't crash or return an error — it returns a wrong answer that looks right. The structured context from the wrong corpus provides just enough coherent background for the model to construct a plausible-sounding response grounded in irrelevant domain knowledge. This is the worst kind of bug: it produces outputs that pass casual inspection.

Per-agent docs_root isolation ensures each agent's KG reflects its actual domain. The shared-corpus model means agents in the same domain share extraction costs automatically, while agents in different domains get clean isolation by construction.

## When to Apply

- When deploying multiple agents that serve different knowledge domains
- When an agent's KG contains entities from an unrelated corpus
- When adding a new agent team (e.g., odds-engine) to an existing multi-agent server
- When KG extraction costs are scaling unexpectedly (shared-corpus dedup reduces N× to 1×)
- When operators need per-agent KG control without modifying the global config

## Examples

### Disable KG for a relay agent (no docs to ingest)

```toml
# ~/.mika/agents/mika-relay/identity.toml
name = "Relay"
emoji = "🔁"

[kg]
enabled = false
```

### Point a trading agent at its own corpus

```toml
# ~/.mika/agents/odds-engine-ceo/identity.toml
name = "OE CEO"
emoji = "📊"

[kg]
docs_root = "/data/odds-engine/docs/solutions"
```

### Keep default behavior (platform engineering agents)

```toml
# ~/.mika/agents/mika-dev/identity.toml
name = "Dev"
emoji = "🛠️"

# [kg] section omitted — defaults to enabled=true, falls back to global chain
```
