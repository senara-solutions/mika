---
title: "Dispatch-trigger label allowlist as config constant"
date: 2026-05-09
type: chore
issue: 1053
branch: chore/1053/mika-dev-allowlist-as-config-constant
modules:
  - crates/mika-agent/src/well_known_agents.rs
tags:
  - dispatch
  - security
  - config
---

## Problem

No allowlist exists for dispatch-triggering label actions. Anyone with write access to a repo can set the `ready` label and trigger autonomous work via mika-dev. Once external contributors arrive, this becomes a vector for unintended autonomous dispatch. The brainstorm (Rec 4 of `mika-platform/docs/brainstorms/2026-05-09-lifecycle-redesign-brainstorm.md`) identifies this as a storage concern — the gate logic that consumes the allowlist is Rec 3 (separate ticket, deferred).

## Decision

**Rust constant in `well_known_agents.rs`**, colocated with the `MIKA_DEV` agent specification. Per operator guidance (lifecycle-redesign brainstorm Rec 4 re-grading): allowlist churn is rare, ship as config constant, core memory hedge is available if churn rises.

Rationale:

1. **Proximity to the consumer.** Rec 3's gate logic will live either as an engine-side intent guard in `agent.rs` (structural, like `webhook_ready_label_dispatch`) or as prompt-level validation in the self-dev skill. Both paths can read a Rust constant. The constant is 3 lines from `MIKA_DEV` — the agent whose behavior it constrains.

2. **Rebuild-required, deployable at quiescent boundary.** Vincent's operational read: allowlist churn is rare (Vincent + `mika-platform-dev` + occasional manual external add). The deploy protocol's § 2 decision matrix governs when restarts are safe. No hot-reload infrastructure needed.

3. **Not core memory.** The brainstorm offered core memory as a "cheap hedge" for if churn rate rises. At zero churn so far, the hedge adds surface area (agent can accidentally overwrite, must be seeded at provision time, must survive compaction) without reducing any real operational cost. The hedge remains available as a future escalation path — if churn rate rises, promote the constant's value to core memory seeding in `provision_well_known_agents()`. The constant itself stays as the authoritative default.

4. **Not skill config / system_prompt.md.** Embedding the allowlist in the self-dev prompt couples storage to one specific consumer (prompt-level validation). The Rust constant is consumer-agnostic — Rec 3 can consume it engine-side or prompt-side without moving the data.

### Gateway gap (noted, not in scope)

The gateway's `format_event_text` for `issues.labeled` events does NOT currently include the `sender.login` in the formatted text delivered to mika-dev. Rec 3 will need to either: (a) add `sender.login` to the formatted event text (gateway change), or (b) have the consumer query the GitHub API for the event's sender. This is Rec 3's problem, not this ticket's. Noted here so the architect can assess whether the storage surface choice constrains Rec 3's options — it does not; a Rust constant is readable from both the gateway and the agent engine.

### Pinned insertion site

**File:** `crates/mika-agent/src/well_known_agents.rs`

The new constant inserts between line 99 (end of `MIKA_DEV` static) and line 101 (start of `MIKA_DEV_IDENTITY` const):

```rust
// line 97:     identity_source: Some(IdentitySource::Static(MIKA_DEV_IDENTITY)),
// line 98:     llm_overrides: &[],
// line 99: };
// line 100: (blank)
// >>> NEW CONSTANT GOES HERE <<<
// line 101: /// mika-dev identity.toml — KG disabled per mika#800.
// line 102: const MIKA_DEV_IDENTITY: &str = "\
```

**Pattern consistency:** The file already contains `pub const` arrays of the same shape:
- `pub const MIKA_ARCH_DISABLED_TOOLS: &[&str]` (line 266) — tools denylist
- `pub const INTRA_PLATFORM_DISPATCH_PEERS: &[&str]` (line 368) — agent names

The new `DISPATCH_TRIGGER_ALLOWLIST` follows the same `pub const NAME: &[&str] = &[...]` pattern. `pub` matches the file's convention — all existing `const` arrays in this file are `pub`.

## Implementation steps

### Step 1 — Add the constant

**File:** `crates/mika-agent/src/well_known_agents.rs`

Insert after line 99 (`};` closing the `MIKA_DEV` static), before line 101 (`MIKA_DEV_IDENTITY`):

```rust
/// Allowlist of GitHub usernames permitted to trigger autonomous dispatch
/// via dispatch-triggering labels (currently: `ready`).
///
/// Consumed by: (future) Rec 3 gate logic — either as an engine-side intent
/// guard in `agent.rs` or as prompt-level validation in the self-dev skill.
///
/// Storage decision: Rust constant per mika#1053 / lifecycle-redesign Rec 4.
/// Churn is rare; rebuild + deploy-at-quiescent-boundary is the operational
/// model. If churn rate rises, promote the value to core memory seeding in
/// `provision_well_known_agents()`.
pub const DISPATCH_TRIGGER_ALLOWLIST: &[&str] = &[
    "samidarko",
    "mika-platform-dev",
];
```

### Step 2 — Add a test

**File:** `crates/mika-agent/src/well_known_agents.rs` (in `#[cfg(test)] mod tests`)

```rust
#[test]
fn dispatch_trigger_allowlist_has_required_defaults() {
    assert!(
        DISPATCH_TRIGGER_ALLOWLIST.contains(&"samidarko"),
        "Vincent must be in the dispatch trigger allowlist"
    );
    assert!(
        DISPATCH_TRIGGER_ALLOWLIST.contains(&"mika-platform-dev"),
        "mika-platform-dev machine user must be in the dispatch trigger allowlist"
    );
    assert!(
        !DISPATCH_TRIGGER_ALLOWLIST.is_empty(),
        "dispatch trigger allowlist must not be empty"
    );
}
```

### Step 3 — Build and verify

```bash
cargo build -p mika-agent
cargo test -p mika-agent -- dispatch_trigger_allowlist
cargo clippy -p mika-agent
```

## What this does NOT include

- **Rec 3 gate logic.** The consumer that checks `issue.labeled.sender` against this allowlist is a separate ticket. This ticket stores the data; that ticket wires the gate.
- **Gateway sender propagation.** Adding `sender.login` to the `format_event_text` output for labeled events is a prerequisite for prompt-level consumption in Rec 3, but not for this storage ticket.
- **Core memory seeding.** The hedge is deferred until churn rate justifies it. The constant is the authoritative source; any future core memory seeding would read from it.
- **Per-label allowlists.** The constant covers all dispatch-triggering labels (currently just `ready`). If different labels need different allowlists, that's a Rec 3 design concern.

## Risks

- **None significant.** This is a 10-line change adding a constant with a test. No runtime behavior changes. No consumers wired up.
