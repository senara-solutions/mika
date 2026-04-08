# Blockchain Task Marketplace for Mika

**Date:** 2026-04-04
**Status:** Brainstorm
**Author:** Vincent / Claude

---

## What We're Building

Mika Cloud as an **autonomous executor** in the decentralized agent economy. External clients post development tasks on-chain (or via x402 HTTP), Mika agents bid/accept, execute in isolated containers, and receive USDC payment on completion. Blockchain is load-bearing for exactly one scenario: **trust across organizational boundaries** where parties don't share infrastructure.

This is not "blockchain integration as a feature." It is Mika Cloud registering as a first-class participant in emerging agent marketplaces, using architecture that already exists (A2A, container isolation, skill system) as the foundation.

---

## Why This Approach

### The Architecture Already Supports This

Three properties make this viable today:

1. **A2A is done.** Mika speaks a standard agent protocol. The Agent Card at `/a2a/{customer_id}/{agent_name}/agent.json` already describes capabilities in a machine-readable format. Any marketplace supporting A2A-compatible task dispatch can reach Mika without custom integration.

2. **Mika Cloud spawns isolated containers on demand.** A marketplace task maps directly onto a task-scoped container: spin up on receipt, execute with only relevant skills loaded, tear down on completion. No context bleed between marketplace and customer workloads. Isolation is structural, not policy-enforced.

3. **The skill system is the capability declaration.** `skill.toml` already describes what an agent can do. Translating the installed skill catalog to a marketplace capability advertisement is a mapping problem, not an architecture problem.

### What Blockchain Actually Provides (and Doesn't)

**Load-bearing for cross-org trust:**
- **Escrow** — client funds locked before Mika starts. No payment risk.
- **Settlement** — completion proof triggers release. No invoices, no disputes.
- **Reputation** — immutable record of completed tasks. Verifiable track record that compounds.
- **Identity** — executor agent has a wallet address. ERC-8004 gives it on-chain identity.

**Not useful for:**
- Tasks between Mika agents within the same customer container (A2A handles this)
- Audit trails within a customer's own instance (SQLite + Langfuse already do this)
- Skill distribution (git-based marketplace is correct)
- Any use case where all parties share a trusted backend

---

## Key Decisions

### 1. Spend Policy: Per-Category Auto-Approve with Telegram Escalation

The spend threshold is per skill category, not a single global value. Skill category is a better risk proxy than budget alone — a $10 refactor has more blast radius than a $25 docs fix.

```toml
[marketplace.spend_policy]
default_auto_approve_usdc = 10

[marketplace.spend_policy.categories]
docs         = { auto_approve_usdc = 25 }
test-gen     = { auto_approve_usdc = 20 }
issue-fix    = { auto_approve_usdc = 10 }
refactor     = { auto_approve_usdc = 5  }   # lower — higher blast radius
code-review  = { auto_approve_usdc = 15 }  # read-only, safer
```

Above threshold: escalate to Vincent via Telegram (existing escalation path). Reputation-based client trust (auto-approve known good clients) is correctly sequenced after Phase 4 when Mika has its own on-chain track record.

### 2. Task Filtering: Curated Mode Only, with Informative Rejections

Only accept tasks matching `skill_categories` in the Agent Card. The asymmetry is decisive: a false rejection (task not taken) costs nothing; a false acceptance (failed execution) creates a permanent on-chain reputation hit. Permissive mode is wrong at any stage, not just v1.

Rejections must be informative via A2A response:
```json
{
  "error": "capabilities_mismatch",
  "detail": "task requires 'database-migration', agent declares ['issue-fix', 'code-review', 'test-gen']"
}
```
This lets marketplaces route cleanly rather than treating it as a generic failure.

### 3. Marketplace Strategy: x402 as Protocol, NEAR as First Directory

x402 is the intake protocol (universal adapter). NEAR AI Agent Market is the first discovery channel. These are layered, not competing:

1. **Implement x402 intake** in the gateway — `402 Payment Required` + payment confirmation webhook. Any x402-speaking client works immediately.
2. **Register on NEAR AI Agent Market** with the Agent Card. NEAR becomes first discovery channel.
3. **Fetch.ai** is a future registry entry (one more registration call against the same infrastructure) when the 48-hour re-registration friction is worth it.

**Open verification needed:** Does NEAR dispatch via A2A JSON-RPC or a proprietary protocol? If A2A, the existing A2A server handler is the intake and x402 is a parallel path. If proprietary, x402 is the universal adapter layer.

### 4. Failure Handling: Full Refund, Mika Eats Compute

If the agent fails (CI red, no viable solution), escrow returns 100% to client. Mika absorbs compute cost.

```
Task lifecycle:
  submitted → working → failed
  └─ escrow: full refund to client
  └─ on-chain: task_failed record (honest)
  └─ Mika cost: container_minutes × cost_per_minute (no revenue)
```

This reinforces curated mode — Mika is structurally incentivized to only accept tasks it can complete. "Did not complete" is a better on-chain record than "completed badly."

### 5. Pricing: Marketplace Revenue Separate from Tier Pricing

Customer tiers (Champion/Runtime/Managed) cover customer workloads. Marketplace revenue is additive and independent:

```
Mika Cloud net = task_budget_usdc - (container_minutes × cost_per_minute)
```

No platform commission on top. Marketplace takes its own native cut (NEAR ~2%, x402 ~0%). Simple, honest, no double-dipping on managed tier customers who also post marketplace tasks.

### 6. Infrastructure Choices

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Chain | Base (Coinbase L2) | x402 native, Agentic Wallets, low fees |
| Settlement token | USDC | Stable, no volatility risk for executor |
| Key management | Coinbase CDP / Agentic Wallets | Never manage private keys in Mika |
| Task intake | A2A + x402 dual pattern | Covers marketplace dispatch and direct HTTP |
| Capability declaration | Agent Card extension | Reuses existing A2A infrastructure |
| Container isolation | Existing Mika Cloud spawning | No new infrastructure |
| Reputation | Passive (from settlement records) | No active staking, no token |
| Custom token | No | YAGNI — USDC sufficient, a Mika token solves no problem today |

---

## The Execution Flow

```
Marketplace (NEAR / x402 client)
         │
         │  Task posted (on-chain or x402 HTTP)
         │  e.g. "fix this GitHub issue, budget: 50 USDC"
         ▼
  Mika Cloud Gateway
         │
         │  Receives via A2A JSON-RPC or x402 HTTP 402 pattern
         │  Validates task against declared skill_categories
         │  Checks budget against per-category spend policy
         │  Rejects with informative error if capabilities_mismatch
         ▼
  Spawn isolated task-scoped agent container
         │
         │  Work item created from task spec (source: marketplace)
         │  Agent executes: plan → implement → PR → CI → close
         ▼
  Task completion (or failure)
         │
         │  Success: artifact (PR URL, diff, results) returned via A2A
         │           on-chain settlement (USDC on Base)
         │           container tears down
         │
         │  Failure: full refund triggered
         │           task_failed record on-chain
         │           container tears down
         ▼
  On-chain reputation update
         (task hash, outcome, payment proof — immutable record)
```

---

## Phased Implementation

### Phase 1 — Agent Card Marketplace Extension (no blockchain code)

Add `capabilities.marketplace` block to Agent Card:
```json
{
  "marketplace": {
    "accepts_external_tasks": true,
    "skill_categories": ["code-review", "issue-fix", "test-generation"],
    "max_task_duration_minutes": 60,
    "spend_policy": { "see": "per-category config above" }
  }
}
```

Register with NEAR AI Agent Market (HTTP POST with card URL). Mika becomes discoverable with zero chain code. Ships independently — has value as a discovery signal before execution works.

### Phase 2 — Task Intake (x402 + A2A)

Implement x402 in the gateway: handle `402 Payment Required`, payment confirmation webhook, container spawn on payment. Implement marketplace routing rule in A2A server: `source: marketplace` triggers task-scoped container instead of routing to customer container.

Both patterns produce the same internal artifact: a `work_item` with `source: marketplace` and `budget_usdc` field. Agent loop is unchanged.

**Concrete cost formula required before this phase ships:**
`Mika Cloud net = task_budget_usdc - (container_minutes × cost_per_minute)`

### Phase 3 — On-chain Settlement

On task close-out, the existing hook gains a new step:
- Submit completion proof to escrow contract (PR URL + task hash)
- Escrow releases USDC to Mika's wallet (`MIKA_WALLET_ADDRESS` config value)
- Settlement receipt written to `audit_events` with `tx_hash`

Settlement is a single HTTPS POST to Coinbase Agentic Wallet API or Base RPC. No chain node, no new crates, no private key management in Mika (delegated to Coinbase CDP).

### Phase 4 — Reputation (Passive)

On-chain reputation is a consequence of Phase 3. Every settled task produces: executor address, task hash, outcome, payment amount, timestamp. ERC-8004 indexes automatically.

Agent Card advertises `reputation.chain: base` pointing to wallet address. Future enhancement: use client reputation symmetrically for tiered auto-approve.

---

## What Is Explicitly NOT in Scope

- **A Mika token.** No tokenomics, no governance token, no speculative asset.
- **Custom smart contracts.** Use existing escrow patterns (NEAR Intents, Coinbase CDP).
- **Our own marketplace.** Mika is an executor participant, not a marketplace operator.
- **On-chain agent logic.** Agent loop stays in Rust, off-chain. Only settlement touches chain.
- **Blockchain for intra-Mika workflows.** A2A handles agent coordination within the platform.

---

## Sequencing Against Current Roadmap

This work is additive, not competitive:

```
Now          Memory architecture re-evaluation
             Knowledge graph
             Behavioral testing / evals

After those  Phase 1: Agent Card marketplace extension   ← low risk, high visibility
             Phase 2: Task intake (x402 + A2A)           ← builds on A2A (done)
             Phase 3: Settlement hook                    ← additive to close-out path
             Phase 4: Reputation                         ← passive consequence
```

Phase 1 can ship independently. Each subsequent phase builds on the previous one. No phase requires the next to deliver value.

---

## Resolved Questions

1. **Spend policy** — Per-category auto-approve thresholds in Agent Card config. Telegram escalation above threshold. Reputation-based trust deferred to post-Phase 4.

2. **Task filtering** — Curated mode only. Informative `capabilities_mismatch` rejections via A2A. Permissive mode is structurally wrong due to asymmetric cost of false acceptance (permanent on-chain reputation damage).

3. **Failure handling** — Full refund on failure. Mika eats compute cost. "Did not complete" beats "completed badly" on-chain.

4. **Which marketplace first** — x402 as universal protocol, NEAR as first directory. Layered, not competing. Verify NEAR dispatch mechanism (A2A vs proprietary) before Phase 2.

5. **Pricing model** — Marketplace revenue independent from tier pricing. `net = budget - compute_cost`. No platform commission. No double-dipping.

---

## Open Questions

1. **NEAR dispatch protocol** — Does NEAR AI Agent Market dispatch via A2A JSON-RPC or a proprietary protocol? Determines whether x402 and A2A are parallel intake paths or layered. Verify before Phase 2 implementation.
