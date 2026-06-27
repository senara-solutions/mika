---
title: "Absent-config-defaults-to-permissive is a contract-leak vector — ship a narrow secure default"
date: 2026-06-27
category: architecture-patterns
module: mika-common/home, mika-agent/skills, well_known_agents
problem_type: best_practice
component: agent-core
severity: high
applies_when:
  - Designing a config block whose absence has a behavioral default (allowlists, feature flags, permission sets)
  - A skill/prompt carries an internal-only output contract (a required suffix line, a verdict keyword) that must not reach a user-facing surface
  - Provisioning a new agent identity, especially via a fallback path that skips an explicit identity template
  - Deciding between a structural fix (secure default) and a downstream output redactor
  - Keeping two hand-maintained copies of a list in sync (a Rust const + an inlined config literal)
tags:
  - identity-allowlist
  - secure-default
  - contract-leak
  - prompt-enforcement-fragile
  - deny-by-default
  - skills
---

# Absent-config-defaults-to-permissive is a contract-leak vector

## Context

mika#1596. Vincent's personal `mika` agent (on `qwen/qwen3.7-max`) appended `Disposition: READY` to **every** reply over Telegram — "How are you?" → "Good, thanks. You? Disposition: READY". `Disposition: READY|ITERATE|ESCALATE` is an internal architect (`mika-arch`) output contract; the model was mimicking it on a user-facing surface.

Root cause: the default agent `identity.toml` written by `bootstrap_fresh_install` (`crates/mika-common/src/home.rs`, the `DEFAULT_IDENTITY` const) had **no `[skills]` block**. `SkillRegistry::apply_identity_allowlist` (`crates/mika-agent/src/skills/mod.rs`) is a **no-op when the allowlist is empty or absent** — so a missing block means *default-permissive*: every bundled skill loads, including the engineering skills (`dev-pilot`, `dev-groom`, `mika-arch-*`, `qa-review*`, `self-dev*`) whose prompts carry the verdict contract. The Mika Cloud family rollout reaches this path: `add-customer.sh` without `--identity-dir` falls through to `bootstrap_fresh_install`, so **every** family customer provisioned that way would inherit the leak.

The well-known dev agents (`mika-dev/qa/arch/relay`) were never affected — they carry explicit `[skills].allowlist`s in `well_known_agents.rs` (see [well-known-agent identity-allowlist migration](well-known-agent-identity-allowlist-migration-2026-05-15.md)). The gap was *only* on the personal/customer fallback path that had no allowlist at all.

## Guidance

**When the absence of a config block has a behavioral default, make that default the safe one — at the source that produces the config, not at a downstream filter.**

The fix (Shape A) added a narrow operator-assistant allowlist to `DEFAULT_IDENTITY`:

```rust
// crates/mika-common/src/home.rs
pub const DEFAULT_AGENT_SKILL_ALLOWLIST: &[&str] = &[
    "calendar", "google-workspace", "browser-control", "desktop",
    "file-reader", "mcp", "web-search", "self-knowledge",
    "shell-exec", "tmux", "git-ops", "gh-read-only",
];
// ...inlined as a [skills].allowlist TOML array in DEFAULT_IDENTITY.
```

An explicit `[skills]` block in a provisioned `identity.toml` still overrides it (`write_default_if_missing` never overwrites an existing file), so the change is backward-compatible — and *already-provisioned* agents keep their old (block-less) identity until re-provisioned, which is why the personal-agent manual fix (add allowlist + restart) was needed for Vincent's existing agent.

Two rejected shapes, and why:
- **Require an explicit `[skills]` block (refuse to start without one).** Adds provisioning friction and a new failure mode on the family path. The point of a secure *default* is that the safe path needs no ceremony.
- **Strip `Disposition:`/`Verdict:` lines from the outbound send path (a redactor).** Treats the symptom. Prompt-borne contracts are fragile (see [prompt-enforcement-structural-guards](../prompt-enforcement-structural-guards.md)); a redactor races every new contract keyword and leaves the engineering skills loaded into a user agent that should never have had them.

**Sub-pattern — keep two hand-maintained list copies in sync with a test.** The fix holds the allowlist in both a Rust const (`DEFAULT_AGENT_SKILL_ALLOWLIST`) and an inlined TOML array inside the `DEFAULT_IDENTITY` string literal. A `home.rs` test parses the *written* `identity.toml` and asserts `parsed_allowlist == DEFAULT_AGENT_SKILL_ALLOWLIST` (order-sensitive) plus the exclusion/inclusion invariants — so drift in either copy fails CI. The cleaner `format!`-render-from-const approach (used by `mika-arch` in `well_known_agents.rs`) is **only available when the identity is runtime-`Computed`**, not a `const &str` consumed directly by `write_default_if_missing`. Converting `DEFAULT_IDENTITY` to a builder/`LazyLock<String>` would ripple to every consumer of the const — not worth it for a backward-compatible const; the test-enforced sync is the lower-risk and actually-more-rigorous choice.

## Why This Matters

A contract-leak is a correctness-and-trust failure on the product's most visible surface (a user's chat). The structural property that caused it — *absence defaults to permissive* — is reusable far beyond this one allowlist: any feature flag, permission set, or capability gate whose "unset" branch grants rather than withholds is one missing provisioning step away from the same class of leak. Deny-by-default is the same instinct applied here as in the [per-method `gh api` deny-by-default matrix](per-method-gh-api-gating-deny-by-default-matrix-2026-06-26.md): the safe outcome is the one you get when you forget to configure anything.

It also reframes "operator misconfiguration" correctly: when a deterministic provisioning path produces the unsafe default, it is a *bug in the default*, not an operator oversight to be papered over with documentation.

## When to Apply

- Any new `identity.toml` section (or analogous config block) whose absence is interpreted as a permissive default — give it a safe default at the writing source.
- Any time a skill or prompt introduces an internal output contract (a required suffix, a verdict keyword): confirm no user-facing agent loads that skill by default. New user-facing skills must be added to `DEFAULT_AGENT_SKILL_ALLOWLIST`; new engineering skills must stay out of it.
- When a fix maintains two copies of a list across a type boundary (const ↔ serialized literal), add a test that parses the produced artifact and asserts equality with the canonical constant — don't trust eyeballing.

## Examples

Before — `DEFAULT_IDENTITY` had no `[skills]` block → `apply_identity_allowlist` no-op → all bundled skills active → `Disposition: READY` leaked to user replies.

After — `DEFAULT_IDENTITY` ships the narrow allowlist → `apply_identity_allowlist` evicts every skill not listed (the engineering skills) at agent load → clean replies; operator-assistant skills (calendar, google-workspace, browser-control, desktop, file-reader, web-search, mcp) preserved. The maintenance coupling to `well_known_agents.rs` and the mika#1595 allowlist↔required_tools coherence guard is documented in code comments and in `crates/mika-agent/CLAUDE.md` (Skills System) + `crates/mika-common/CLAUDE.md` (Home directory).
