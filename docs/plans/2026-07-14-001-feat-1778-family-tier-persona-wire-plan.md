---
issue: 1778
type: feat
scope: mika-common (home.rs)
title: family-tier persona wire — MIKA_AGENT_TIER env-gate + FAMILY_SOUL/FAMILY_IDENTITY
---

# Plan — mika#1778: family-tier persona wire

## Problem

Fresh Mika containers always receive the operator persona (`DEFAULT_SOUL`, English, executive-assistant tone, orchestrator-scoped skill allowlist including `github`/`git-ops`/`shell-exec`). A non-technical family member's first hour is charabia.

Vincent-approved family-tier persona (native French, `tu` register, warm/patient/simple, zero technical jargon) exists at the operator's `~/.claude/plans/family-tier-persona-flo.md` (approved 2026-07-12). It is the gate for Sonia's family onboarding, scheduled 2026-07-14.

**Deadline:** family persona must be live before Sonia's visit today.

## Root cause

`crates/mika-common/src/home.rs::bootstrap()` unconditionally writes `DEFAULT_IDENTITY` and `DEFAULT_SOUL`. No tier concept.

## Committed position — env-var-gated tier constants

- **`MIKA_AGENT_TIER`** env var, case-insensitive. Recognized `"default"` / `"family"`; unknown → default + `warn!` (visible in `MIKA_SPIRIT_LOG_FILE`).
- **`AgentTier` enum** (`Default`/`Family`) with `AgentTier::from_env()` — reads the env var once.
- **`FAMILY_SOUL`, `FAMILY_IDENTITY`, `FAMILY_AGENT_SKILL_ALLOWLIST`** constants added to `crates/mika-common/src/home.rs`.
- `bootstrap()` calls `AgentTier::from_env()`, then writes tier-selected `identity.toml` + `soul.md` via existing `write_default_if_missing` (contract preserved: existing files never overwritten).

The env-var was chosen over a `bootstrap()` parameter because:
- It composes with mika-cloud's per-customer container provisioning (samidarko sets `MIKA_AGENT_TIER=family` in the container's `.env` at provision time — no code changes in the caller).
- It matches the existing pattern of `MIKA_DEV_MODE`, `MIKA_DISABLE_AGENT_PROVISIONING` (env-gated startup behavior).
- It stays fail-open: any misconfigured or missing var lands the operator persona (safe default).

## Scope

### In scope for v1 (this PR)

- New `AgentTier` enum + `from_env()` at the top of `crates/mika-common/src/home.rs`.
- Modify `bootstrap()` to consult the tier when picking identity/soul templates. Other files (`config.toml`, `heartbeat.md`, `user.md`) are tier-agnostic.
- New `FAMILY_SOUL` constant — Vincent's approved French persona verbatim + a `## First-turn opening` reference section (persona-file concern; not auto-emitted at runtime).
- New `FAMILY_IDENTITY` constant — TOML template with `[skills].allowlist` narrow to family-appropriate surfaces.
- New `FAMILY_AGENT_SKILL_ALLOWLIST: &[&str]` constant — allowlist single source of truth, kept in sync with the TOML array via a test assertion (same pattern as `DEFAULT_AGENT_SKILL_ALLOWLIST`).
- 5 new tests:
  - `test_bootstrap_writes_family_persona_when_tier_family` (`#[serial]`)
  - `test_bootstrap_writes_default_persona_when_tier_unset` (`#[serial]`)
  - `test_bootstrap_writes_default_persona_on_unknown_tier` (`#[serial]`)
  - `test_family_allowlist_matches_family_identity_toml`
  - `test_agent_tier_from_env_variants` (`#[serial]`)
- Mark `test_bootstrap_fresh_install_writes_narrow_skill_allowlist` `#[serial]` + defensive `remove_var` to prevent race with new family-tier serial tests.
- Update root `CLAUDE.md` (`MIKA_AGENT_TIER` in Optional startup behavior section).
- Update `crates/mika-common/CLAUDE.md` (Home directory section).

### Deferred / out of scope

- **mika-cloud `add-customer.sh --tier` flag** — companion PR; samidarko owns the caller-side env-var wire during Flo/Sonia provisioning.
- **Runtime tier switching** — `bootstrap()` only fires on fresh install. Already-provisioned containers keep their persona regardless of env-var changes. This is intentional (safer) — retrofitting existing containers is a follow-up ticket.
- **`vous` register variant** — Vincent's plan noted `vous` may suit older family members; defer as follow-up. Default `tu` for v1.
- **Multiple family personas per person** — v1 has a single canonical family persona; per-person adaptation (name, context) happens at provisioning via `user.md`.
- **Deploy pipeline change** — this PR ships the binary; the family persona lands when a new container image built from post-merge main is provisioned with the env var. No changes to release/deploy tooling required.

## Acceptance criteria

- [x] **AC1** — `AgentTier` enum defined with `Default` / `Family` variants. `AgentTier::from_env()` reads `MIKA_AGENT_TIER` case-insensitively (`trim().to_ascii_lowercase()`). Values not in `{"", "default", "family"}` fall through to `Default` with a single `warn!` log line naming the offending value.
- [x] **AC2** — `FAMILY_SOUL` and `FAMILY_IDENTITY` constants exist and reproduce Vincent's approved persona verbatim. `FAMILY_SOUL` carries both the persona block AND the first-greeting text under a distinct `## First-turn opening` section (available as context but not auto-emitted at runtime — that's a runtime concern, not a persona-file concern).
- [x] **AC3** — `FAMILY_AGENT_SKILL_ALLOWLIST` contains exactly `calendar`, `google-workspace`, `file-reader`, `web-search`, `desktop`, `browser-control`. Excludes all dev/orchestrator skills. `FAMILY_IDENTITY` TOML array matches the constant (kept-in-sync test asserts this).
- [x] **AC4** — `bootstrap()` calls `AgentTier::from_env()`, then writes `identity.toml` and `soul.md` from the tier-selected constants (not the `DEFAULT_*` constants when tier is `Family`). Existing files still preserved by `write_default_if_missing` (contract unchanged).
- [x] **AC5** — Unit tests pass: (a) `test_bootstrap_writes_family_persona_when_tier_family` sets env, bootstraps, asserts `soul.md` contains `chaleureux, patient, simple` AND `identity.toml` excludes `"github"`; (b) `test_bootstrap_writes_default_persona_when_tier_unset` clears env, bootstraps, asserts `soul.md` contains `senior executive assistant`; (c) `test_bootstrap_writes_default_persona_on_unknown_tier` sets `MIKA_AGENT_TIER=quantum`, asserts default; (d) `test_family_allowlist_matches_family_identity_toml` mirrors the existing default-allowlist assertion pattern; (e) `test_agent_tier_from_env_variants` covers unset/empty/case/whitespace/unknown paths. All new env-var-touching tests use `#[serial]`.
- [x] **AC6** — Root `CLAUDE.md` and `crates/mika-common/CLAUDE.md` reference `MIKA_AGENT_TIER` with recognized-values list + fall-through behavior.
- [x] **AC7** — `cargo test -p mika-common home::tests` — 26/26 pass; `cargo build -p mika-common` clean; `cargo fmt --package mika-common` clean.

## Definition of Done

- All AC satisfied.
- No `Disposition:`/`Verdict:` leakage — the family allowlist is architecturally narrower than the default (which itself excludes those skills post-mika#1596).
- No secrets in constants (persona text is public; Telegram bot token wiring is out of scope).
- Merge unblocks samidarko's caller-side provisioning: build fresh container image from post-merge main + set `MIKA_AGENT_TIER=family` in Sonia's per-customer container `.env` at first startup.

## Files touched

- `crates/mika-common/src/home.rs` — `AgentTier` enum, `FAMILY_*` constants, `bootstrap()` tier branch, 5 new tests + 1 test hardening.
- `crates/mika-common/CLAUDE.md` — Home directory section: family-tier reference.
- `CLAUDE.md` (root) — Optional startup behavior: `MIKA_AGENT_TIER` entry.
- `docs/plans/2026-07-14-001-feat-1778-family-tier-persona-wire-plan.md` (this file).

## Verification

```bash
cargo test -p mika-common home::tests             # 26/26 pass
cargo build -p mika-common                        # clean
cargo fmt --package mika-common --check           # clean
```

## Deploy dependency (answer to samidarko-claude's provisioning question)

**Yes, a new mika-agent container image (built from post-merge main) is required for the family persona to be available.** The `bootstrap()` logic lives in the compiled binary; `MIKA_AGENT_TIER=family` at provisioning time is necessary but not sufficient — the binary must contain the code path. Sequence for Sonia today:
1. This PR merges to main.
2. release.yml builds a fresh mika-agent binary/image (auto on tag; can be manually kicked if needed).
3. samidarko provisions Sonia's container using the new image, setting `MIKA_AGENT_TIER=family` in the container's `.env` before first startup.
4. First `bootstrap()` on that container writes `FAMILY_IDENTITY` + `FAMILY_SOUL`.

If the release/deploy chain is not aligned with today's deadline, an alternative same-hour path exists: samidarko builds mika-agent from the merged main locally, tags a private image, and uses it for Sonia's container. That's an operator-side decision (samidarko's call, not this PR's scope).

## References

- Vincent-approved persona: `~/.claude/plans/family-tier-persona-flo.md` (operator host, samidarko-Claude)
- Deadline anchor: Sonia + Nicolas + Benjamin visit 2026-07-14 (today)
- Related mika#1596 — `DEFAULT_AGENT_SKILL_ALLOWLIST` leak class; family allowlist follows the same shape (narrow, kept-in-sync via test)
- Related mika-cloud — companion `add-customer.sh --tier` flag is a follow-up
- mika-platform samidarko-claude directive 2026-07-14 morning — direct-implement authorization (loop's dispatch side still stalled)

Plan: docs/plans/2026-07-14-001-feat-1778-family-tier-persona-wire-plan.md
