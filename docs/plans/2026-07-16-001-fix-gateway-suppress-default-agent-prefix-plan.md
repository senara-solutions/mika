---
issue: (family-tier launch hotfix — filed alongside this PR)
type: fix
scope: crates/mika-gateway/src/routes.rs
title: suppress `[mika]` prefix on outbound Telegram for single-agent (family-tier) customers
---

# Plan — mika-gateway: suppress default-agent prefix on outbound Telegram

## Problem

During the family-tier launch on 2026-07-16 the persona wire fired correctly (French, warm, `tu`, 🌸) but every outbound Telegram message from the family agent was prefixed with `[mika]`:

```
[mika] Bonjour Nicolas 🌸 Je suis Mika…
```

For a non-technical family member (Vincent's aunt, cousins) this is a cosmetic parasite that breaks first-hour-Perfect: the very first character she sees is a bracketed label she can't parse. Vincent surfaced this immediately after the first cousin's greeting.

Zero data loss risk — the prefix is added by the gateway on the outbound rendering path only. The agent's own DB (`messages` table) stores the message content without the prefix; the `outbound_messages` reply-routing table still records `agent_name` in a proper column. **This is a rendering-only fix.**

## Root cause

`crates/mika-gateway/src/routes.rs` `handle_send` unconditionally prepends `[<agent_name>] ` when the `/send` payload carries an `agent_name`:

```rust
let text_to_send = match &payload.agent_name {
    Some(name) => {
        owned_text = format!("[{name}] {}", payload.text);
        &owned_text
    }
    None => &payload.text,
};
```

The comment reads "for identification in multi-agent setups". That intent is right — but the rule needs to know when the container has only one agent. Family-tier customers have exactly one agent named `mika` (`mika_common::agent::DEFAULT_AGENT`); multi-agent customers name their additional agents things like `work-mika`, `personal-mika`. So we can distinguish by name.

## Committed position — suppress prefix only for the default agent name

- Extract the render decision into a pure module-private helper `format_outbound_text(agent_name: Option<&str>, text: &str) -> String` so the branch is unit-testable without HTTP scaffolding.
- Rule: if `agent_name == mika_common::agent::DEFAULT_AGENT`, do NOT prefix. Any other name (or `None`) uses the pre-existing shape:
  - `Some(name)` where `name != "mika"` → `format!("[{name}] {text}")` (unchanged)
  - `Some("mika")` → raw text (new)
  - `None` → raw text (unchanged)
- Case-sensitive match against the constant. Uppercase/mixed-case `agent_name` values already fail the existing validation in `handle_send` (only lowercase alphanumeric + hyphens), so case-folding here would be dead code.

### Why not other shapes

- **Tier-based suppression** (`if customer.tier == "family"`) would add a DB lookup on the outbound hot path and only cover family — it would leave BYOK/managed single-agent customers still prefixed. The default-agent-name test is O(1), no DB round-trip, and covers every single-agent customer regardless of tier.
- **Container agent-count check** would require the gateway to know the containeragent inventory. Doable but adds a cross-service call and cache invalidation. The name-based test is materially simpler for the same behavior.
- **Agent-side suppression** (omit `agent_name` from `/send` payload when tier is family) fixes only new agents, not existing ones. Gateway-side is one deploy that covers everyone instantly.

## Scope

### In scope

- `crates/mika-gateway/src/routes.rs`:
  - New module-private `format_outbound_text()` pure helper
  - `handle_send` calls it in place of the inline match
  - 4 new unit tests in `routes::tests`
- `crates/mika-gateway/CLAUDE.md` — update § Agent Identification & Reply Routing to note the default-agent suppression + the launch incident that motivated it

### Deliberately out of scope

- **Agent-side changes** — the agent still sends `agent_name` in the payload; that's needed for `outbound_messages` reply routing (unaffected by the render suppression).
- **Existing conversations retrofit** — the fix is rendering-only, so:
  - **Non-destructive by construction.** Not one byte of any agent's DB is touched by this fix.
  - **No re-roll of existing agents needed** — the fresh gateway pod applies the suppression to ALL family customers immediately on rollout, including Flo, Nicolas, Benjamin, Litha, and any others already provisioned.
- **`parse_agent_prefix` on inbound** — kept tolerant of both prefixed and unprefixed shapes for backwards compat with pre-fix history + non-default multi-agent customers. No changes needed here.
- **Wizard-language English seam** — separate BEAUTY gap flagged in the first-hour walk-through; not launch-blocking.

## Acceptance criteria

- [x] **AC1** — `format_outbound_text(Some("mika"), "Bonjour")` returns `"Bonjour"` (no prefix). Enforces the default-agent suppression. Asserted by `format_outbound_text_suppresses_default_agent_prefix`.
- [x] **AC2** — `format_outbound_text(Some("work-mika"), "hi")` returns `"[work-mika] hi"` (multi-agent identification preserved). Asserted by `format_outbound_text_keeps_prefix_for_named_agents` across three canonical names (`work-mika`, `personal-mika`, `mika-dev`).
- [x] **AC3** — `format_outbound_text(None, "raw")` returns `"raw"` (backwards-compat for operator-only /send without identification). Asserted by `format_outbound_text_no_prefix_when_agent_name_absent`.
- [x] **AC4** — Case-sensitive match: `format_outbound_text(Some("Mika"), "x")` starts with `[` (prefix retained; uppercase names would be rejected earlier by `handle_send` validation anyway). Asserted by `format_outbound_text_default_agent_match_is_case_sensitive`.
- [x] **AC5** — Non-destructive: helper produces a `String` (no mutation of input). DB and outbound_messages recording paths unchanged.
- [x] **AC6** — Full gateway test suite (`cargo test -p mika-gateway --bin mika-gateway`) passes 274/274. New tests (4) + existing (270).
- [x] **AC7** — `cargo build`, `cargo clippy --all-targets`, `cargo fmt --check` all clean.

## Definition of Done

- All AC satisfied.
- `crates/mika-gateway/CLAUDE.md` reflects the suppression rule + the founding-incident anchor.
- No behavioral change for BYOK/managed multi-agent customers (their `[work-mika]`-shape prefix still lands).
- No agent-side changes, no DB migrations, no state touched — pure rendering-layer edit.

## Deploy plan (post-merge, operator-driven)

1. Build+push new `mika-gateway` image from post-merge main.
2. `kubectl set image deployment/mika-gateway -n mika-system-dev mika-gateway=<new-tag>` OR `helm upgrade mika-gateway helm/mika-gateway --set image.tag=<new-tag>` (preferred — no drift).
3. Rollout status. Gateway is HPA-scaled 2-10 with `maxUnavailable: 0`, so zero-downtime.
4. From then, every family (and single-agent BYOK/managed) customer's next outbound Telegram message ships without the `[mika]` prefix. Existing agents unaffected in every other way.

**Non-destructive confirmation**: conversations live in per-container SQLite DBs on PVCs — the gateway rollout doesn't touch any agent pod. Every family member (Flo already provisioned, Nicolas/Benjamin/Litha whether already-live or upcoming) keeps their `messages`/`llm_calls`/`core_memory`/etc. exactly as-is.

## Scope answer to samidarko + Vincent

**Scope of the fix: gateway-side rendering only.**
- New provisions → no `[mika]` prefix (immediate on gateway rollout)
- Existing family agents (Flo + any cousins already onboarded) → no `[mika]` prefix on their NEXT outbound message (immediate on gateway rollout)
- **No agent re-roll needed for any existing customer.** The `helm upgrade` on Flo I ran yesterday was for the persona wire (that lives in the agent binary + env + PVC files); this fix is entirely different plumbing (gateway `/send` render).
- **Non-destructive** — the agent's DB is not touched by the gateway; the message content the LLM produces goes into the agent's `messages` table without any `[mika]` prefix (the prefix is added only when the gateway relays to Telegram's `sendMessage` API). Flo's/Nicolas's/Benjamin's/Litha's conversations are preserved by construction.

## References

- **Founding incident:** 2026-07-16 07:00 UTC — Vincent-reported first-hour parasite; first cousin's `[mika] Bonjour !`
- **Adjacent PRs:** mika-cloud#167 (pairing UX), mika-cloud#169 (tier wire), mika#1778/#1779 (persona code)
- **Related trail:** mika-gateway `outbound_messages` table + `parse_agent_prefix()` — the reply-routing paths this fix intentionally leaves untouched
- **Design principle:** rendering-layer suppression, not schema/state change → cousins' conversations preserved by construction

Plan: docs/plans/2026-07-16-001-fix-gateway-suppress-default-agent-prefix-plan.md
